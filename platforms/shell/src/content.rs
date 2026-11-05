//! In-process rendering demo using the layout team's HTML -> paint -> frame API.
//! The demo evaluates its page script once per document load before painting.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock, mpsc};
use std::thread;

use browser::{BrowserRuntime, Page};
use html::NodeID;
use layout::{FontMetrics, LayoutTree, Size, layout_document};
use render::{BitmapFont, Compositor, Frame, OutlineFont};
use tiny_skia::Pixmap;
use webapis::{AlertSink, ConsoleSink, ViewportSize};

struct NoopConsole;

impl ConsoleSink for NoopConsole {
    fn log(&self, _message: &str) {}
}

/// The system font all page text uses, set once by [`load_page_font`].
static PAGE_FONT: OnceLock<Option<OutlineFont>> = OnceLock::new();

/// Loads the first of `files` that parses as a font as the page font.
///
/// Each platform shell lists its default serif font first, since browsers
/// show pages without author fonts in a serif face. Only the first call has
/// an effect. If no file loads, or this is never called (as in unit tests),
/// pages use the built-in bitmap font.
pub(crate) fn load_page_font(files: Vec<PathBuf>) {
    PAGE_FONT.get_or_init(|| {
        let candidates = files.len();
        let font = OutlineFont::load_first(files);
        if font.is_some() {
            log::debug!("page font loaded (first loadable of {candidates} candidate files)");
        } else {
            log::warn!(
                "no page font could be loaded from {candidates} candidate files; using the built-in bitmap font"
            );
        }
        font
    });
}

/// The loaded page font, or `None` to use the built-in bitmap font.
///
/// `font-family` is not supported yet, so every element shares this font.
fn page_font() -> Option<&'static OutlineFont> {
    PAGE_FONT.get().and_then(Option::as_ref)
}

/// The metrics layout uses, matching the font [`render_document`] draws with.
fn page_metrics() -> &'static dyn FontMetrics {
    match page_font() {
        Some(font) => font,
        None => &BitmapFont,
    }
}

/// Renders `page` into a `width` × `height` device-pixel frame, first
/// evaluating its script if that has not happened yet. `scale_factor` is
/// device pixels per CSS pixel.
#[cfg(test)]
fn render_demo(
    page: &browser::Page,
    width: u32,
    height: u32,
    scale_factor: f64,
    alert_sink: Arc<dyn AlertSink>,
    runtime: &BrowserRuntime,
    scripts_evaluated: &mut bool,
) -> Frame {
    if !*scripts_evaluated {
        // The completion value is not part of painting. `alert` returns
        // `undefined`, so requiring the welcome string aborts the first draw
        // and the window never opens.
        evaluate_page_script(page, Arc::clone(&alert_sink), runtime.clone());
        *scripts_evaluated = true;
    }
    render_document(
        page,
        width,
        height,
        scale_factor,
        alert_sink,
        runtime,
        scripts_evaluated,
    )
}

/// Lays out and paints `page`, unscrolled, into a `width` × `height`
/// device-pixel frame, first evaluating its script if that has not happened
/// yet.
#[cfg(test)]
fn render_document(
    page: &Page,
    width: u32,
    height: u32,
    scale_factor: f64,
    alert_sink: Arc<dyn AlertSink>,
    runtime: &BrowserRuntime,
    scripts_evaluated: &mut bool,
) -> Frame {
    if !*scripts_evaluated {
        evaluate_page_script(page, alert_sink, runtime.clone());
        *scripts_evaluated = true;
    }
    let layout = PageLayout::for_device(page, width, height, scale_factor);
    compose_frame(&layout, width, height, scale_factor, 0.0)
}

/// Paints `layout` scrolled down by `scroll_y` CSS pixels into a
/// `width` × `height` device-pixel frame.
///
/// With a system page font, text is rasterized at device resolution. The
/// bitmap fallback renders at one device pixel per CSS pixel, so its frame is
/// the CSS-pixel viewport size and [`render_page`] scales it up. The scroll
/// offset is snapped to whole frame pixels so text does not shimmer while
/// scrolling.
fn compose_frame(
    layout: &PageLayout,
    width: u32,
    height: u32,
    scale_factor: f64,
    scroll_y: f32,
) -> Frame {
    let (css_width, css_height) = layout.css_size;
    match page_font() {
        Some(font) => {
            let scale = scale_factor as f32;
            let scroll_y = (scroll_y * scale).round() / scale;
            let display_list = paint::paint_document_scrolled(&layout.tree, scroll_y);
            Compositor::new(width, height).compose_paint_with_font(&display_list, font, scale)
        }
        None => {
            let display_list = paint::paint_document_scrolled(&layout.tree, scroll_y.round());
            Compositor::new(css_width, css_height).compose_paint(&display_list)
        }
    }
}

/// Returns the CSS-pixel viewport for a `width` × `height` device-pixel page
/// area at `scale_factor` device pixels per CSS pixel.
pub(crate) fn css_viewport(width: u32, height: u32, scale_factor: f64) -> (u32, u32) {
    (
        (width as f64 / scale_factor).ceil() as u32,
        (height as f64 / scale_factor).ceil() as u32,
    )
}

/// A page parsed, styled, and laid out for one CSS-pixel viewport.
///
/// The window keeps this between frames so scrolling only repaints; it is
/// rebuilt when the page or the viewport size changes. Painting and hit
/// testing share it so they agree on geometry.
pub(crate) struct PageLayout {
    /// The viewport width and height in CSS pixels.
    css_size: (u32, u32),
    tree: LayoutTree,
}

impl PageLayout {
    /// Lays out `page` for a `width` × `height` CSS-pixel viewport.
    pub(crate) fn new(page: &Page, width: u32, height: u32) -> Self {
        let document = html::parse_raw_html(page.html.to_owned());
        let stylesheets: Vec<_> = page
            .stylesheets
            .iter()
            .map(|source| css::parse_stylesheet(source))
            .collect();
        let styles = css::style_document(&document, &stylesheets);
        let tree = layout_document(
            &document,
            &styles,
            Size {
                width: width as f32,
                height: height as f32,
            },
            page_metrics(),
        );
        Self {
            css_size: (width, height),
            tree,
        }
    }

    /// Lays out `page` for a `width` × `height` device-pixel page area at
    /// `scale_factor` device pixels per CSS pixel.
    #[cfg(test)]
    pub(crate) fn for_device(page: &Page, width: u32, height: u32, scale_factor: f64) -> Self {
        let (css_width, css_height) = css_viewport(width, height, scale_factor);
        Self::new(page, css_width, css_height)
    }

    /// The viewport width and height, in CSS pixels, this layout is for.
    pub(crate) fn css_size(&self) -> (u32, u32) {
        self.css_size
    }

    /// How far the viewport can scroll down, in CSS pixels; zero when the
    /// whole document fits.
    pub(crate) fn max_scroll(&self) -> f32 {
        (self.tree.scroll_height() - self.css_size.1 as f32).max(0.0)
    }

    /// Returns the node under a viewport point in CSS pixels while the page
    /// is scrolled down by `scroll_y`, or `None` when the point hits no
    /// element box.
    ///
    /// Node IDs match the document a [`PageEvents`] session loads from the
    /// same page, because both parse `page.html` the same way.
    pub(crate) fn hit_test(&self, x: f32, y: f32, scroll_y: f32) -> Option<NodeID> {
        self.tree.hit_test(x, y + scroll_y)
    }
}

/// Evaluates the page's script for its side effects, including `alert`.
///
/// The window runs scripts through [`PageEvents`] instead, so listeners
/// outlive the script; tests use this to render a page after its script ran.
#[cfg(test)]
fn evaluate_page_script(page: &Page, alert_sink: Arc<dyn AlertSink>, runtime: BrowserRuntime) {
    log::debug!("evaluating page script for {}", page.url);
    let controller = browser::request_controller(None);
    runtime
        .open_page(
            page,
            Arc::new(NoopConsole),
            alert_sink,
            controller,
            browser::security_context_for_url(&page.url),
        )
        .expect("the page script should execute");
}

enum PageCommand {
    Load {
        page: Page,
        alert_sink: Arc<dyn AlertSink>,
        runtime: BrowserRuntime,
        viewport: Option<ViewportSize>,
    },
    Resize(ViewportSize),
    Click(Option<NodeID>),
}

/// Owns the page's JavaScript session on a thread that is not the UI thread.
///
/// `alert` waits until the UI thread shows the dialog. Running the script or
/// a click listener on the UI thread would deadlock that wait. Commands are
/// applied in order, so a click is delivered only after the load that
/// registered its listener has finished.
pub(crate) struct PageEvents {
    commands: mpsc::Sender<PageCommand>,
}

impl PageEvents {
    /// Starts the session thread. It exits when the last sender is dropped.
    pub(crate) fn new() -> Self {
        let (commands, incoming) = mpsc::channel();
        thread::spawn(move || run_page_events(incoming));
        Self { commands }
    }

    /// Replaces the live document with `page` and runs its script.
    #[allow(dead_code)]
    pub(crate) fn load(
        &self,
        page: &Page,
        alert_sink: Arc<dyn AlertSink>,
        runtime: BrowserRuntime,
    ) {
        self.load_with_viewport(page, alert_sink, runtime, None);
    }

    /// Replaces the live document and exposes `viewport` to page scripts.
    pub(crate) fn load_with_viewport(
        &self,
        page: &Page,
        alert_sink: Arc<dyn AlertSink>,
        runtime: BrowserRuntime,
        viewport: Option<ViewportSize>,
    ) {
        if self
            .commands
            .send(PageCommand::Load {
                page: page.clone(),
                alert_sink,
                runtime,
                viewport,
            })
            .is_err()
        {
            log::warn!("page session is not running; dropped load of {}", page.url);
        }
    }

    /// Dispatches a `click` at `target`, or at the document when the click
    /// hit no element. `target` comes from [`PageLayout::hit_test`].
    ///
    /// A click sent before the first load finishes is applied after that
    /// load, because the session thread handles one command at a time.
    pub(crate) fn click(&self, target: Option<NodeID>) {
        if self.commands.send(PageCommand::Click(target)).is_err() {
            log::warn!("page session is not running; dropped click");
        }
    }

    /// Updates the viewport dimensions exposed to the live page session.
    pub(crate) fn resize(&self, viewport: ViewportSize) {
        if self.commands.send(PageCommand::Resize(viewport)).is_err() {
            log::warn!("page session is not running; dropped resize");
        }
    }
}

fn run_page_events(commands: mpsc::Receiver<PageCommand>) {
    let mut session = None;
    while let Ok(command) = commands.recv() {
        match command {
            PageCommand::Load {
                page,
                alert_sink,
                runtime,
                viewport,
            } => {
                log::debug!("page session loading {}", page.url);
                let controller = browser::request_controller(None);
                let result = match viewport {
                    Some(viewport) => runtime.open_page_with_viewport(
                        &page,
                        Arc::new(NoopConsole),
                        alert_sink,
                        controller,
                        browser::security_context_for_url(&page.url),
                        viewport,
                    ),
                    None => runtime.open_page(
                        &page,
                        Arc::new(NoopConsole),
                        alert_sink,
                        controller,
                        browser::security_context_for_url(&page.url),
                    ),
                };
                match result {
                    Ok(loaded) => session = Some(loaded),
                    Err(error) => log::debug!("page script failed: {error}"),
                }
            }
            PageCommand::Resize(viewport) => {
                if let Some(session) = &mut session {
                    session.set_viewport(viewport);
                }
            }
            PageCommand::Click(target) => {
                log::trace!("page click dispatched: {target:?}");
                let Some(session) = &session else {
                    continue;
                };
                let result = match target {
                    Some(node) => session.dispatch_click_at(node),
                    None => session.dispatch_click(),
                };
                if let Err(error) = result {
                    log::warn!("page click failed: {error}");
                }
            }
        }
    }
}

/// Renders `layout`, scrolled down by `scroll_y` CSS pixels, into a
/// `width` × `height` device-pixel pixmap sized for the page card.
///
/// Text drawn with the system page font is rasterized at device resolution;
/// the bitmap fallback renders at 1x and is scaled up with nearest-neighbor
/// sampling so it keeps a consistent logical size on Retina. When the
/// document is taller than the viewport, an overlay scrollbar thumb shows
/// the scroll position. Returns `None` for an empty area.
pub(crate) fn render_page(
    layout: &PageLayout,
    width: u32,
    height: u32,
    scale_factor: f64,
    scroll_y: f32,
) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(width, height)?;
    log::trace!("rendering page {width}x{height} at scale {scale_factor}, scroll {scroll_y}");
    let frame = compose_frame(layout, width, height, scale_factor, scroll_y);
    // Device pixels per frame pixel: 1 when the frame is already at device
    // resolution, otherwise the window's scale for the 1x bitmap fallback.
    let frame_scale = if frame.width == width {
        1.0
    } else {
        scale_factor
    };
    for (y, row) in pixmap
        .data_mut()
        .chunks_exact_mut(width as usize * 4)
        .enumerate()
    {
        let source_y = ((y as f64 / frame_scale) as u32).min(frame.height - 1);
        for (x, pixel) in row.chunks_exact_mut(4).enumerate() {
            let source_x = ((x as f64 / frame_scale) as u32).min(frame.width - 1);
            let offset = (source_y * frame.width + source_x) as usize * 4;
            let rgba = &frame.pixels[offset..offset + 4];
            // Composite RGBA onto the white page so the card stays opaque.
            let alpha = u32::from(rgba[3]);
            let channel =
                |value: u8| ((u32::from(value) * alpha + 255 * (255 - alpha)) / 255) as u8;
            pixel.copy_from_slice(&[channel(rgba[0]), channel(rgba[1]), channel(rgba[2]), 255]);
        }
    }
    draw_scrollbar(&mut pixmap, layout, scale_factor, scroll_y);
    Some(pixmap)
}

/// Draws an overlay scrollbar thumb along the right edge of the page when
/// the document can scroll. Sizes are CSS pixels scaled to the device.
fn draw_scrollbar(pixmap: &mut Pixmap, layout: &PageLayout, scale_factor: f64, scroll_y: f32) {
    const THUMB_WIDTH: f32 = 6.0;
    const MARGIN: f32 = 3.0;
    const MIN_THUMB: f32 = 24.0;
    let max_scroll = layout.max_scroll();
    if max_scroll <= 0.0 {
        return;
    }
    let scale = scale_factor as f32;
    let track = pixmap.height() as f32 - 2.0 * MARGIN * scale;
    let viewport = layout.css_size.1 as f32;
    let thumb_height = (track * viewport / (viewport + max_scroll))
        .max(MIN_THUMB * scale)
        .min(track);
    let fraction = (scroll_y / max_scroll).clamp(0.0, 1.0);
    let Some(thumb) = crate::toolbar::rounded_rect(
        pixmap.width() as f32 - (THUMB_WIDTH + MARGIN) * scale,
        MARGIN * scale + fraction * (track - thumb_height),
        THUMB_WIDTH * scale,
        thumb_height,
        THUMB_WIDTH * scale / 2.0,
    ) else {
        return;
    };
    let mut paint = tiny_skia::Paint::default();
    paint.set_color_rgba8(0, 0, 0, 90);
    paint.anti_alias = true;
    pixmap.fill_path(
        &thumb,
        &paint,
        tiny_skia::FillRule::Winding,
        tiny_skia::Transform::identity(),
        None,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    struct SilentAlert;

    impl AlertSink for SilentAlert {
        fn alert(&self, _message: &str) {}
    }

    fn test_alert_sink() -> Arc<dyn AlertSink> {
        Arc::new(SilentAlert)
    }

    fn test_runtime() -> BrowserRuntime {
        BrowserRuntime::in_memory()
    }

    fn welcome_color(page: &Page, property: &str) -> [u8; 4] {
        let sheet = css::parse_stylesheet(&page.stylesheets[0]);
        let value = sheet
            .rules
            .iter()
            .find_map(|rule| {
                rule.declarations
                    .iter()
                    .find(|item| item.property == property)
            })
            .expect("welcome color declaration");
        let color = css::parse_color(&value.value).expect("supported welcome color");
        [color.r, color.g, color.b, color.a]
    }

    #[test]
    fn demo_produces_layout_background_and_renderer_text() {
        let page = browser::welcome_page();
        let runtime = test_runtime();
        let mut scripts_evaluated = false;
        let frame = render_demo(
            &page,
            160,
            120,
            1.0,
            test_alert_sink(),
            &runtime,
            &mut scripts_evaluated,
        );
        assert_eq!((frame.width, frame.height), (160, 120));
        // `body { margin: 8px }` leaves the canvas visible at the corner, and
        // `main` starts inside that margin.
        assert_eq!(&frame.pixels[..4], &[255; 4]);
        let inside_main = (10 * 160 + 10) * 4;
        assert_eq!(
            &frame.pixels[inside_main..inside_main + 4],
            &welcome_color(&page, "background-color")
        );
        assert!(
            frame.pixels[..160 * 100 * 4]
                .chunks_exact(4)
                .any(|p| p == welcome_color(&page, "color"))
        );
        assert_eq!(&frame.pixels[160 * 110 * 4..160 * 110 * 4 + 4], &[255; 4]);
    }

    #[test]
    fn page_is_scaled_to_device_pixels() {
        let page_source = browser::welcome_page();
        let runtime = test_runtime();
        evaluate_page_script(&page_source, test_alert_sink(), runtime.clone());
        for scale in [1.0, 1.5, 2.0] {
            let (width, height) = ((160.0 * scale) as u32, (120.0 * scale) as u32);
            let layout = PageLayout::for_device(&page_source, width, height, scale);
            let page = render_page(&layout, width, height, scale, 0.0).unwrap();
            assert_eq!((page.width(), page.height()), (width, height));
            let data = page.data();
            assert_eq!(&data[..4], &[255; 4]);
            let inside_main =
                ((10.0 * scale) as usize * width as usize + (10.0 * scale) as usize) * 4;
            assert_eq!(
                &data[inside_main..inside_main + 4],
                &welcome_color(&page_source, "background-color")
            );
            assert!(
                data.chunks_exact(4)
                    .any(|p| p == welcome_color(&page_source, "color"))
            );
            let bottom = ((110.0 * scale) as u32 * width) as usize * 4;
            assert_eq!(&data[bottom..bottom + 4], &[255; 4]);
        }
    }

    #[test]
    fn a_click_after_load_runs_the_document_listener() {
        struct Recording {
            messages: std::sync::Mutex<Vec<String>>,
            arrived: std::sync::mpsc::Sender<()>,
        }

        impl AlertSink for Recording {
            fn alert(&self, message: &str) {
                self.messages.lock().unwrap().push(message.to_owned());
                let _ = self.arrived.send(());
            }
        }

        let (arrived_tx, arrived_rx) = std::sync::mpsc::channel();
        let alert = Arc::new(Recording {
            messages: std::sync::Mutex::new(Vec::new()),
            arrived: arrived_tx,
        });
        let events = PageEvents::new();
        events.load(
            &Page {
                url: String::new(),
                html: "<main><script src=\"script.js\"></script></main>".into(),
                stylesheets: Vec::new(),
                scripts: vec![
                    "function callback() { alert('Clicked') }\n\
                     document.addEventListener('click', callback)"
                        .into(),
                ],
            },
            alert.clone(),
            test_runtime(),
        );
        events.click(None);

        arrived_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("document click should alert");
        assert_eq!(alert.messages.lock().unwrap().as_slice(), ["Clicked"]);
    }

    #[test]
    fn clicking_the_welcome_button_runs_its_listener() {
        struct Recording {
            messages: std::sync::Mutex<Vec<String>>,
            arrived: std::sync::mpsc::Sender<()>,
        }

        impl AlertSink for Recording {
            fn alert(&self, message: &str) {
                self.messages.lock().unwrap().push(message.to_owned());
                let _ = self.arrived.send(());
            }
        }

        let page = browser::welcome_page();
        let (width, height) = (640, 480);
        let layout = PageLayout::new(&page, width, height);
        let tree = &layout.tree;
        let button = html::parse_raw_html(page.html.clone())
            .get_element_by_id("btn")
            .expect("welcome page has #btn");
        fn find(layout_box: &layout::LayoutBox, node: NodeID) -> Option<layout::Rect> {
            if layout_box.node == Some(node) {
                return Some(layout_box.rect);
            }
            let in_lines = layout_box.lines.iter().flat_map(|line| &line.fragments);
            let atomics = in_lines.filter_map(|fragment| match fragment {
                layout::LineFragment::Atomic(atomic) => Some(atomic),
                _ => None,
            });
            layout_box
                .children
                .iter()
                .chain(atomics)
                .find_map(|child| find(child, node))
        }
        let rect = find(&tree.root, button).expect("#btn is laid out");
        let (x, y) = (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
        assert_eq!(
            layout.hit_test(x, y, 0.0),
            Some(button),
            "the button's center should hit the button"
        );

        let (arrived_tx, arrived_rx) = std::sync::mpsc::channel();
        let alert = Arc::new(Recording {
            messages: std::sync::Mutex::new(Vec::new()),
            arrived: arrived_tx,
        });
        let events = PageEvents::new();
        events.load(&page, alert.clone(), test_runtime());
        events.click(layout.hit_test(x, y, 0.0));

        arrived_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("button click should alert");
        assert_eq!(alert.messages.lock().unwrap().as_slice(), ["Clicked"]);
    }

    #[test]
    fn tiny_and_empty_pages_are_safe() {
        let page = browser::welcome_page();
        let layout = PageLayout::for_device(&page, 0, 10, 1.0);
        assert!(render_page(&layout, 0, 10, 1.0, 0.0).is_none());
        for (width, height) in [(1, 1), (3, 48), (17, 70)] {
            let layout = PageLayout::for_device(&page, width, height, 2.0);
            let max_scroll = layout.max_scroll();
            for scroll_y in [0.0, max_scroll / 2.0, max_scroll] {
                assert!(render_page(&layout, width, height, 2.0, scroll_y).is_some());
            }
        }
    }

    fn long_page() -> Page {
        Page {
            url: String::new(),
            html: "<body><div id=\"top\"></div><div id=\"bottom\"></div></body>".into(),
            stylesheets: vec![
                "body { margin: 0 } div { height: 300px } \
                 #top { background-color: red } #bottom { background-color: blue }"
                    .into(),
            ],
            scripts: Vec::new(),
        }
    }

    #[test]
    fn scrolling_shows_lower_content_and_shifts_hit_testing() {
        let page = long_page();
        let layout = PageLayout::new(&page, 160, 120);
        assert_eq!(layout.css_size(), (160, 120));
        assert_eq!(layout.max_scroll(), 600.0 - 120.0);

        let pixel = |pixmap: &Pixmap, x: u32, y: u32| {
            let offset = (y * pixmap.width() + x) as usize * 4;
            pixmap.data()[offset..offset + 4].to_vec()
        };
        let top = render_page(&layout, 160, 120, 1.0, 0.0).unwrap();
        assert_eq!(pixel(&top, 10, 60), [255, 0, 0, 255]);
        let scrolled = render_page(&layout, 160, 120, 1.0, 400.0).unwrap();
        assert_eq!(pixel(&scrolled, 10, 60), [0, 0, 255, 255]);

        let document = html::parse_raw_html(page.html.clone());
        let bottom = document.get_element_by_id("bottom").unwrap();
        let top_node = document.get_element_by_id("top").unwrap();
        assert_eq!(layout.hit_test(10.0, 60.0, 0.0), Some(top_node));
        assert_eq!(layout.hit_test(10.0, 60.0, 400.0), Some(bottom));
    }

    #[test]
    fn the_scrollbar_thumb_appears_only_on_scrollable_pages_and_tracks_position() {
        // Darkened pixels in the rightmost columns are the overlay thumb.
        let thumb_rows = |pixmap: &Pixmap| -> Vec<u32> {
            (0..pixmap.height())
                .filter(|y| {
                    let offset = (y * pixmap.width() + pixmap.width() - 6) as usize * 4;
                    pixmap.data()[offset] < 200
                })
                .collect()
        };
        let short = Page {
            url: String::new(),
            html: "<body>short</body>".into(),
            stylesheets: Vec::new(),
            scripts: Vec::new(),
        };
        let layout = PageLayout::new(&short, 160, 120);
        assert_eq!(layout.max_scroll(), 0.0);
        assert!(thumb_rows(&render_page(&layout, 160, 120, 1.0, 0.0).unwrap()).is_empty());

        let mut page = long_page();
        page.stylesheets = vec!["body { margin: 0 } div { height: 300px }".into()];
        let layout = PageLayout::new(&page, 160, 120);
        let at_top = thumb_rows(&render_page(&layout, 160, 120, 1.0, 0.0).unwrap());
        let at_end = thumb_rows(&render_page(&layout, 160, 120, 1.0, 480.0).unwrap());
        assert!(!at_top.is_empty() && !at_end.is_empty());
        assert!(at_top[0] < 10, "thumb starts at the top: {at_top:?}");
        assert!(
            *at_end.last().unwrap() > 110,
            "thumb ends at the bottom: {at_end:?}"
        );
    }

    #[test]
    fn rendered_page_scripts_use_the_alert_sink() {
        #[derive(Default)]
        struct TestAlert(std::sync::Mutex<Vec<String>>);

        impl AlertSink for TestAlert {
            fn alert(&self, message: &str) {
                self.0.lock().unwrap().push(message.to_owned());
            }
        }

        let alert = Arc::new(TestAlert::default());
        let page = Page {
            url: String::new(),
            html: "<main><script src=\"script.js\"></script></main>".into(),
            stylesheets: Vec::new(),
            scripts: vec!["alert('rendered page')".into()],
        };

        let runtime = test_runtime();
        let mut scripts_evaluated = false;
        render_document(
            &page,
            160,
            120,
            1.0,
            alert.clone(),
            &runtime,
            &mut scripts_evaluated,
        );

        assert_eq!(alert.0.lock().unwrap().as_slice(), ["rendered page"]);
    }

    #[test]
    fn welcome_page_alert_still_paints() {
        #[derive(Default)]
        struct Recording(std::sync::Mutex<Vec<String>>);

        impl AlertSink for Recording {
            fn alert(&self, message: &str) {
                self.0.lock().unwrap().push(message.to_owned());
            }
        }

        let welcome = browser::welcome_page();
        let page = Page {
            url: welcome.url,
            html: welcome.html,
            stylesheets: welcome.stylesheets,
            scripts: vec!["alert('Hello from script.js')".into()],
        };
        let alert = Arc::new(Recording::default());
        let runtime = test_runtime();
        let mut scripts_evaluated = false;
        let frame = render_demo(
            &page,
            160,
            120,
            1.0,
            alert.clone(),
            &runtime,
            &mut scripts_evaluated,
        );

        assert!(scripts_evaluated);
        assert_eq!(alert.0.lock().unwrap().as_slice(), ["Hello from script.js"]);
        assert_eq!((frame.width, frame.height), (160, 120));
        let inside_main = (10 * 160 + 10) * 4;
        assert_eq!(
            &frame.pixels[inside_main..inside_main + 4],
            &welcome_color(&page, "background-color")
        );
    }

    #[test]
    fn page_pixels_follow_html_text_and_css_colors() {
        let runtime = test_runtime();
        let mut scripts_evaluated = true;
        let mut page = Page {
            url: String::new(),
            html: "<main>Hello</main>".into(),
            stylesheets: vec!["main { background-color: #abc; color: red; }".into()],
            scripts: Vec::new(),
        };
        let first = render_document(
            &page,
            160,
            120,
            1.0,
            test_alert_sink(),
            &runtime,
            &mut scripts_evaluated,
        );
        assert_eq!(&first.pixels[..4], &[170, 187, 204, 255]);
        assert!(first.pixels.chunks_exact(4).any(|p| p == [255, 0, 0, 255]));

        page.html = "<main>World</main>".into();
        let changed_text = render_document(
            &page,
            160,
            120,
            1.0,
            test_alert_sink(),
            &runtime,
            &mut scripts_evaluated,
        );
        assert_ne!(first.pixels, changed_text.pixels);

        page.stylesheets = vec!["main { background-color: rgb(18, 52, 86); color: #abc; }".into()];
        let changed_css = render_document(
            &page,
            160,
            120,
            1.0,
            test_alert_sink(),
            &runtime,
            &mut scripts_evaluated,
        );
        assert_eq!(&changed_css.pixels[..4], &[18, 52, 86, 255]);
        assert!(
            changed_css
                .pixels
                .chunks_exact(4)
                .any(|p| p == [170, 187, 204, 255])
        );
    }

    #[test]
    fn user_agent_canvas_and_text_colors_reach_the_frame() {
        let runtime = test_runtime();
        let mut scripts_evaluated = true;
        let mut page = Page {
            url: String::new(),
            html: "<html><body><main>Hi</main></body></html>".into(),
            stylesheets: Vec::new(),
            scripts: Vec::new(),
        };
        let frame = render_document(
            &page,
            160,
            120,
            1.0,
            test_alert_sink(),
            &runtime,
            &mut scripts_evaluated,
        );
        assert_eq!(&frame.pixels[..4], &[255, 255, 255, 255]);
        assert!(frame.pixels.chunks_exact(4).any(|p| p == [0, 0, 0, 255]));

        page.stylesheets = vec!["html { background-color: #123456; color: red; }".into()];
        let frame = render_document(
            &page,
            160,
            120,
            1.0,
            test_alert_sink(),
            &runtime,
            &mut scripts_evaluated,
        );
        assert_eq!(&frame.pixels[..4], &[18, 52, 86, 255]);
        assert!(frame.pixels.chunks_exact(4).any(|p| p == [255, 0, 0, 255]));
    }
}
