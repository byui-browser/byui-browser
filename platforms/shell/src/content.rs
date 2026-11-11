//! In-process rendering demo using the layout team's HTML -> paint -> frame API.
//! The demo evaluates its page script once per document load before painting.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread;

use browser::{BrowserRuntime, Page};
use html::NodeID;
use layout::{FontMetrics, LayoutTree, Size, layout_document};
use render::{BitmapFont, Compositor, Frame, OutlineFont};
use tiny_skia::Pixmap;
use webapis::{AlertSink, ConsoleSink, LocationNavigate, ViewportSize};

pub(crate) struct DevtoolsConsole(pub(crate) Arc<Mutex<devtools::Console>>);

impl ConsoleSink for DevtoolsConsole {
    fn log(&self, message: &str) {
        if let Ok(mut console) = self.0.lock() {
            console.log(devtools::Level::Log, message);
        }
    }

    fn warn(&self, message: &str) {
        if let Ok(mut console) = self.0.lock() {
            console.log(devtools::Level::Warn, message);
        }
    }

    fn error(&self, message: &str) {
        if let Ok(mut console) = self.0.lock() {
            console.log(devtools::Level::Error, message);
        }
    }
}

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
#[cfg(test)]
fn compose_frame(layout: &PageLayout, width: u32, height: u32, scale: f64, scroll: f32) -> Frame {
    compose_frame_with_find(layout, width, height, scale, scroll, None, false)
}

fn compose_frame_with_find(
    layout: &PageLayout,
    width: u32,
    height: u32,
    scale_factor: f64,
    scroll_y: f32,
    find: Option<&crate::find_in_page::FindInPage>,
    dark: bool,
) -> Frame {
    let (css_width, css_height) = layout.css_size;
    let mut backgrounds = layout.text_backgrounds(find, dark);
    let mut display_list = |scroll| {
        if backgrounds.is_empty() {
            paint::paint_document_scrolled(&layout.tree, scroll)
        } else {
            paint::paint_document_scrolled_with_text_backgrounds(&layout.tree, scroll, |text| {
                backgrounds
                    .remove(&(text as *const layout::TextFragment))
                    .unwrap_or_default()
            })
        }
    };
    match page_font() {
        Some(font) => {
            let scale = scale_factor as f32;
            let scroll_y = (scroll_y * scale).round() / scale;
            let display_list = display_list(scroll_y);
            Compositor::new(width, height).compose_paint_with_font(&display_list, font, scale)
        }
        None => {
            let display_list = display_list(scroll_y.round());
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

    /// Text actually present in layout, including offscreen fragments, with CSS bounds.
    /// Block contexts stay separate. Soft-wrapped lines supply a word-space boundary.
    pub(crate) fn find_text(&self) -> Vec<crate::find_in_page::TextRun> {
        self.find_fragments()
            .into_iter()
            .map(|(t, boundary, new_line)| crate::find_in_page::TextRun {
                text: t.text.clone(),
                rect: t.rect,
                boundary,
                new_line,
            })
            .collect()
    }

    /// References stay tied to this layout; frame-local identity links search ranges
    /// to Paint's borrowed fragments without guessing from text or coordinates.
    fn find_fragments(&self) -> Vec<(&layout::TextFragment, bool, bool)> {
        fn collect<'a>(
            b: &'a layout::LayoutBox,
            out: &mut Vec<(&'a layout::TextFragment, bool, bool)>,
        ) {
            let mut boundary = true;
            for (line_index, line) in b.lines.iter().enumerate() {
                let mut first = true;
                for fragment in &line.fragments {
                    match fragment {
                        layout::LineFragment::Text(t) if !t.text.is_empty() => {
                            out.push((t, boundary, first && line_index > 0));
                            boundary = false;
                            first = false;
                        }
                        layout::LineFragment::Atomic(child) => {
                            collect(child, out);
                            boundary = true;
                        }
                        _ => {}
                    }
                }
            }
            for child in &b.children {
                collect(child, out);
            }
        }
        let mut out = Vec::new();
        collect(&self.tree.root, &mut out);
        out
    }

    fn text_backgrounds(
        &self,
        find: Option<&crate::find_in_page::FindInPage>,
        dark: bool,
    ) -> std::collections::HashMap<*const layout::TextFragment, Vec<paint::PaintBox>> {
        let mut result: std::collections::HashMap<_, Vec<_>> = std::collections::HashMap::new();
        let Some(find) = find else {
            return result;
        };
        let fragments = self.find_fragments();
        for (range, color) in find.highlight_ranges(dark) {
            let Some((fragment, _, _)) = fragments.get(range.run) else {
                continue;
            };
            if let Some(rect) = text_range_rect(fragment, range.bytes, page_metrics()) {
                result
                    .entry(*fragment as *const layout::TextFragment)
                    .or_default()
                    .push(paint::PaintBox {
                        rect,
                        background: Some(color),
                    });
            }
        }
        result
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

/// Advance-cell bounds measured with the exact font used by layout and rendering.
/// These are text selection cells, not glyph ink outlines. No character-count estimate.
fn text_range_rect(
    fragment: &layout::TextFragment,
    range: std::ops::Range<usize>,
    metrics: &dyn FontMetrics,
) -> Option<layout::Rect> {
    let prefix = fragment.text.get(..range.start)?;
    let matched = fragment.text.get(range)?;
    let x = fragment.rect.x + metrics.text_width(prefix, fragment.font_size);
    let width = metrics.text_width(matched, fragment.font_size);
    (width > 0.0).then_some(layout::Rect {
        x,
        y: fragment.rect.y,
        width,
        height: fragment.rect.height,
    })
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
        console: Arc<dyn ConsoleSink>,
        alert_sink: Arc<dyn AlertSink>,
        runtime: BrowserRuntime,
        device_pixel_ratio: f64,
        viewport: Option<ViewportSize>,
        navigate: LocationNavigate,
    },
    Resize(ViewportSize),
    ScrollY(f64),
    DevicePixelRatio(f64),
    Click(Option<NodeID>),
    Snapshot(mpsc::Sender<String>),
}

/// Owns the page's JavaScript session on a thread that is not the UI thread.
///
/// `alert` waits until the UI thread shows the dialog. Running the script or
/// a click listener on the UI thread would deadlock that wait. Commands are
/// applied in order, so a click is delivered only after the load that
/// registered its listener has finished.
pub(crate) struct PageEvents {
    commands: mpsc::Sender<PageCommand>,
    /// The error that stopped the most recently loaded page's scripts, if
    /// any. Written by the session thread, read by the Compatibility audit.
    script_error: Arc<Mutex<Option<String>>>,
}

impl PageEvents {
    /// Starts the session thread. It exits when the last sender is dropped.
    pub(crate) fn new() -> Self {
        let (commands, incoming) = mpsc::channel();
        let script_error = Arc::new(Mutex::new(None));
        let errors = Arc::clone(&script_error);
        thread::spawn(move || run_page_events(incoming, &errors));
        Self {
            commands,
            script_error,
        }
    }

    /// The error that stopped the current page's scripts, or `None` when
    /// they finished (or have not run yet).
    pub(crate) fn script_error(&self) -> Option<String> {
        self.script_error.lock().map_or(None, |error| error.clone())
    }

    /// Replaces the live document with `page` and runs its script.
    #[allow(dead_code)]
    pub(crate) fn load(
        &self,
        page: &Page,
        alert_sink: Arc<dyn AlertSink>,
        runtime: BrowserRuntime,
        device_pixel_ratio: f64,
    ) {
        self.load_with_viewport(
            page,
            Arc::new(NoopConsole),
            alert_sink,
            runtime,
            device_pixel_ratio,
            None,
            Arc::new(|_, _| {}),
        );
    }

    /// Replaces the live document and exposes `device_pixel_ratio` and
    /// `viewport` (CSS pixels) to page scripts. `Location` navigation
    /// requests from the page are passed to `navigate` on the session thread.
    pub(crate) fn load_with_viewport(
        &self,
        page: &Page,
        console: Arc<dyn ConsoleSink>,
        alert_sink: Arc<dyn AlertSink>,
        runtime: BrowserRuntime,
        device_pixel_ratio: f64,
        viewport: Option<ViewportSize>,
        navigate: LocationNavigate,
    ) {
        if self
            .commands
            .send(PageCommand::Load {
                page: page.clone(),
                console,
                alert_sink,
                runtime,
                device_pixel_ratio,
                viewport,
                navigate,
            })
            .is_err()
        {
            log::warn!("page session is not running; dropped load of {}", page.url);
        }
    }

    /// Updates the current page's output device scale after a display change.
    pub(crate) fn set_device_pixel_ratio(&self, device_pixel_ratio: f64) {
        if self
            .commands
            .send(PageCommand::DevicePixelRatio(device_pixel_ratio))
            .is_err()
        {
            log::warn!("page session is not running; dropped device scale update");
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

    /// Updates the vertical scroll offset exposed to the live page session.
    pub(crate) fn set_scroll_y(&self, scroll_y: f32) {
        if self
            .commands
            .send(PageCommand::ScrollY(scroll_y as f64))
            .is_err()
        {
            log::warn!("page session is not running; dropped scroll position update");
        }
    }

    /// Returns a serialization of the current live DOM after scripts mutate it.
    pub(crate) fn snapshot_html(&self) -> Option<String> {
        let (sender, receiver) = mpsc::channel();
        self.commands.send(PageCommand::Snapshot(sender)).ok()?;
        receiver
            .recv_timeout(std::time::Duration::from_millis(250))
            .ok()
    }
}

fn run_page_events(commands: mpsc::Receiver<PageCommand>, script_error: &Mutex<Option<String>>) {
    let mut session = None;
    while let Ok(command) = commands.recv() {
        match command {
            PageCommand::Load {
                page,
                console,
                alert_sink,
                runtime,
                device_pixel_ratio,
                viewport,
                navigate,
            } => {
                log::debug!("page session loading {}", page.url);
                // A previous page's failure must not be blamed on this one.
                set_script_error(script_error, None);
                let controller = browser::request_controller(None);
                match runtime.open_page_with_viewport_and_navigation(
                    &page,
                    console,
                    alert_sink,
                    controller,
                    browser::security_context_for_url(&page.url),
                    device_pixel_ratio,
                    viewport,
                    navigate,
                ) {
                    Ok(loaded) => {
                        session = Some(loaded);
                        set_script_error(script_error, None);
                    }
                    Err(error) => {
                        log::debug!("page script failed: {error}");
                        set_script_error(script_error, Some(error.to_string()));
                    }
                }
            }
            PageCommand::Resize(viewport) => {
                if let Some(session) = &mut session {
                    session.set_viewport(viewport);
                }
            }
            PageCommand::ScrollY(scroll_y) => {
                if let Some(session) = &mut session {
                    session.set_scroll_y(scroll_y);
                }
            }
            PageCommand::DevicePixelRatio(device_pixel_ratio) => {
                if let Some(session) = &session
                    && let Err(error) = session.set_device_pixel_ratio(device_pixel_ratio)
                {
                    log::warn!("could not update window.devicePixelRatio: {error}");
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
            PageCommand::Snapshot(sender) => {
                let html = session
                    .as_ref()
                    .map_or_else(String::new, browser::PageSession::document_html);
                let _ = sender.send(html);
            }
        }
    }
}

fn set_script_error(slot: &Mutex<Option<String>>, error: Option<String>) {
    if let Ok(mut slot) = slot.lock() {
        *slot = error;
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
#[cfg(test)]
pub(crate) fn render_page(
    layout: &PageLayout,
    width: u32,
    height: u32,
    scale: f64,
    scroll: f32,
) -> Option<Pixmap> {
    render_page_with_find(layout, width, height, scale, scroll, None, false)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_page_with_find(
    layout: &PageLayout,
    width: u32,
    height: u32,
    scale_factor: f64,
    scroll_y: f32,
    find: Option<&crate::find_in_page::FindInPage>,
    dark: bool,
) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(width, height)?;
    log::trace!("rendering page {width}x{height} at scale {scale_factor}, scroll {scroll_y}");
    let frame = compose_frame_with_find(layout, width, height, scale_factor, scroll_y, find, dark);
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
        for (x, pixel) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
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

    #[cfg(target_os = "macos")]
    #[test]
    fn actual_outline_font_highlights_use_the_same_advances_as_rendering() {
        let font = OutlineFont::load_first([
            "/System/Library/Fonts/Supplemental/Times New Roman.ttf",
            "/System/Library/Fonts/Times.ttc",
            "/System/Library/Fonts/Helvetica.ttc",
        ])
        .expect("macOS page font");
        let doc = html::parse_raw_html("<body><p>WiWi</p></body>".into());
        let styles = css::style_document(&doc, &[]);
        let tree = layout_document(
            &doc,
            &styles,
            Size {
                width: 300.0,
                height: 100.0,
            },
            &font,
        );
        let fragment = tree
            .paint_order()
            .into_iter()
            .find_map(|p| {
                if let layout::PaintItem::Text(t) = p {
                    Some(t)
                } else {
                    None
                }
            })
            .unwrap();
        let rect = text_range_rect(fragment, 1..2, &font).unwrap();
        assert_eq!(
            rect.x,
            fragment.rect.x + font.advance('W', fragment.font_size)
        );
        assert_eq!(rect.width, font.advance('i', fragment.font_size));
        assert!(rect.width < font.advance('W', fragment.font_size));
        for scale in [1.0, 1.5, 2.0] {
            let plain = paint::paint_document_scrolled(&tree, 0.0);
            let highlighted =
                paint::paint_document_scrolled_with_text_backgrounds(&tree, 0.0, |t| {
                    if std::ptr::eq(t, fragment) {
                        vec![paint::PaintBox {
                            rect,
                            background: Some(paint::Color {
                                r: 255,
                                g: 220,
                                b: 0,
                                a: 140,
                            }),
                        }]
                    } else {
                        vec![]
                    }
                });
            let compositor = Compositor::new((300.0 * scale) as u32, (100.0 * scale) as u32);
            let before = compositor.compose_paint_with_font(&plain, &font, scale);
            let after = compositor.compose_paint_with_font(&highlighted, &font, scale);
            assert_ne!(before.pixels, after.pixels);
            for (i, (a, b)) in before
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .zip(after.pixels.as_chunks::<4>().0.iter())
                .enumerate()
            {
                if a != b {
                    let x = (i % after.width as usize) as f32;
                    assert!(x >= (rect.x * scale).round() && x < (rect.right() * scale).round());
                }
            }
        }
    }

    #[test]
    fn highlight_geometry_uses_real_proportional_advances() {
        struct Proportional;
        impl FontMetrics for Proportional {
            fn advance(&self, c: char, size: f32) -> f32 {
                match c {
                    'W' => size,
                    'i' => size / 5.0,
                    _ => size / 2.0,
                }
            }
            fn vertical_metrics(&self, size: f32) -> layout::VerticalMetrics {
                layout::VerticalMetrics {
                    ascent: size * 0.8,
                    descent: size * 0.2,
                    line_gap: 0.0,
                }
            }
        }
        let doc = html::parse_raw_html("<body>WiWi</body>".into());
        let styles = css::style_document(&doc, &[]);
        let tree = layout_document(
            &doc,
            &styles,
            Size {
                width: 300.0,
                height: 100.0,
            },
            &Proportional,
        );
        let fragment = tree
            .paint_order()
            .into_iter()
            .find_map(|p| {
                if let layout::PaintItem::Text(t) = p {
                    Some(t)
                } else {
                    None
                }
            })
            .unwrap();
        let rect = text_range_rect(fragment, 1..2, &Proportional).unwrap();
        assert_eq!(rect.x, fragment.rect.x + fragment.font_size);
        assert_eq!(rect.width, fragment.font_size / 5.0);
        assert!(rect.width < fragment.rect.width / 4.0);
        assert_eq!(rect.y, fragment.rect.y);
        assert!(text_range_rect(fragment, 0..0, &Proportional).is_none());
    }

    #[test]
    fn backgrounds_follow_each_wrapped_fragment_and_no_find_preserves_pixels() {
        use crate::find_in_page::FindInPage;
        let page = browser::Page {
            url: "byui://test".into(),
            html: "<body><p>Hello <span>world</span> again hello world</p><p>WiWi</p></body>"
                .into(),
            stylesheets: vec![],
            scripts: vec![],
        };
        for width in [95, 300] {
            let layout = PageLayout::new(&page, width, 100);
            let mut find = FindInPage::default();
            find.replace_runs(layout.find_text());
            find.insert("hello world");
            assert_eq!(find.count(), "1/2");
            let boxes = layout.text_backgrounds(Some(&find), false);
            assert!(boxes.len() >= 2);
            for (fragment, _, _) in layout.find_fragments() {
                if let Some(backgrounds) = boxes.get(&(fragment as *const layout::TextFragment)) {
                    for background in backgrounds {
                        assert!(background.rect.x >= fragment.rect.x);
                        assert!(background.rect.right() <= fragment.rect.right() + 0.001);
                        assert_eq!(background.rect.y, fragment.rect.y);
                    }
                }
            }
            let empty = FindInPage::default();
            assert_eq!(
                compose_frame(&layout, width, 100, 1.0, 0.0).pixels,
                compose_frame_with_find(&layout, width, 100, 1.0, 0.0, Some(&empty), false).pixels
            );
            let plain = compose_frame(&layout, width, 100, 1.0, 0.0);
            let highlighted =
                compose_frame_with_find(&layout, width, 100, 1.0, 0.0, Some(&find), false);
            assert_ne!(plain.pixels, highlighted.pixels);
            // Opaque dark glyph pixels remain dark; backgrounds are never painted over glyphs.
            for (a, b) in plain
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .zip(highlighted.pixels.as_chunks::<4>().0.iter())
            {
                if a[..3] == [0, 0, 0] {
                    assert_eq!(a, b);
                }
            }
            for scroll in [0.0, layout.max_scroll()] {
                let mut boxes = layout.text_backgrounds(Some(&find), false);
                let list = paint::paint_document_scrolled_with_text_backgrounds(
                    &layout.tree,
                    scroll,
                    |t| {
                        boxes
                            .remove(&(t as *const layout::TextFragment))
                            .unwrap_or_default()
                    },
                );
                assert!(
                    list.items
                        .iter()
                        .any(|p| matches!(p, paint::DisplayItem::Text { .. }))
                );
            }
            for scale in [1.0, 1.5, 2.0] {
                let viewport = PageLayout::for_device(&page, 300, 200, scale);
                find.replace_runs(viewport.find_text());
                assert_eq!(find.count(), "1/2");
                assert!(
                    render_page_with_find(&viewport, 300, 200, scale, 0.0, Some(&find), true)
                        .is_some()
                );
            }
        }
    }

    #[test]
    fn find_uses_layout_text_excludes_hidden_content_and_tracks_reflow() {
        use crate::find_in_page::FindInPage;
        let page = browser::Page { url: "https://example.com/".into(),
            html: "<html><head><title>secret</title><script>secret</script><style>.gone {display:none}</style></head><body><p>Hello <span>world</span> again</p><p class='gone'>secret</p><p>world</p></body></html>".into(),
            stylesheets: vec![".gone {display:none}".into()], scripts: vec![] };
        let mut find = FindInPage::default();
        let wide = PageLayout::new(&page, 600, 100);
        find.replace_runs(wide.find_text());
        find.insert("world");
        assert_eq!(find.count(), "1/2");
        find.select_all();
        find.insert("secret");
        assert_eq!(find.count(), "0/0");
        find.select_all();
        find.insert("hello world again");
        assert_eq!(find.count(), "1/1");
        let narrow = PageLayout::new(&page, 90, 100);
        find.replace_runs(narrow.find_text());
        assert_eq!(find.count(), "1/1");
        assert!(narrow.find_text().len() >= wide.find_text().len());
        find.select_all();
        find.insert("again");
        let narrow_y = find.target().unwrap().y;
        find.replace_runs(wide.find_text());
        assert!(narrow_y > find.target().unwrap().y);
        assert_eq!(
            find.scroll_target(wide.max_scroll()),
            Some(find.target().unwrap().y.min(wide.max_scroll()))
        );
        find.replace_runs(Vec::new());
        assert_eq!(find.count(), "0/0");
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
                .as_chunks::<4>()
                .0
                .iter()
                .copied()
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
                data.as_chunks::<4>()
                    .0
                    .iter()
                    .copied()
                    .any(|p| p == welcome_color(&page_source, "color"))
            );
            let bottom = ((110.0 * scale) as u32 * width) as usize * 4;
            assert_eq!(&data[bottom..bottom + 4], &[255; 4]);
        }
    }

    #[test]
    fn location_navigation_requests_reach_the_browser_loader_boundary() {
        let (navigate_tx, navigate_rx) = std::sync::mpsc::channel();
        let events = PageEvents::new();
        let page = Page {
            url: "https://example.test/start".into(),
            html: "<script>location.assign('/next')</script>".into(),
            stylesheets: Vec::new(),
            scripts: vec!["location.assign('/next')".into()],
        };
        events.load_with_viewport(
            &page,
            Arc::new(NoopConsole),
            test_alert_sink(),
            test_runtime(),
            1.0,
            None,
            Arc::new(move |url, behavior| {
                let _ = navigate_tx.send((url, behavior));
            }),
        );

        let (url, behavior) = navigate_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("Location.assign should reach the navigation callback");
        assert_eq!(url.as_str(), "https://example.test/next");
        assert_eq!(behavior, webapis::NavigationBehavior::Auto);
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
            1.0,
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
        events.load(&page, alert.clone(), test_runtime(), 1.0);
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
        assert!(
            first
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .copied()
                .any(|p| p == [255, 0, 0, 255])
        );

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
                .as_chunks::<4>()
                .0
                .iter()
                .copied()
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
        assert!(
            frame
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .copied()
                .any(|p| p == [0, 0, 0, 255])
        );

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
        assert!(
            frame
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .copied()
                .any(|p| p == [255, 0, 0, 255])
        );
    }
}
