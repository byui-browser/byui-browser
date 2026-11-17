//! In-process rendering demo using the layout team's HTML -> paint -> frame API.
//! The demo evaluates its page script once per document load before painting.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread;
use std::time::Duration;

use browser::{BrowserRuntime, Page};
use devtools::performance::{EventCategory, PerformanceLog, TimelineEvent};
use html::{NodeID, NodeKind};
use layout::{FontMetrics, ImageKey, LayoutTree, Size, layout_document_with_images};
use render::{Compositor, FontCollection, Frame, ImageStore, OutlineFont};
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

/// The decoded `<img>` files of the page decoded last, kept so relayouts
/// (window resizes, zoom) reuse them instead of decoding again. Pages are
/// recognized by their shared image list (`Arc::ptr_eq`), so a clone of the
/// same loaded page hits and a newly loaded page replaces the entry.
type DecodedImages = (Arc<[browser::ImageResource]>, Arc<ImageStore>);
static DECODED_IMAGES: Mutex<Option<DecodedImages>> = Mutex::new(None);

/// Decodes `page`'s downloaded images into a store keyed by their `src`,
/// reusing the previous result for the same page.
fn decoded_images(page: &Page) -> Arc<ImageStore> {
    let resources = &page.resources.images;
    if resources.is_empty() {
        return Arc::new(ImageStore::new());
    }
    let mut cached = DECODED_IMAGES
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some((list, store)) = cached.as_ref()
        && Arc::ptr_eq(list, resources)
    {
        return Arc::clone(store);
    }
    let mut store = ImageStore::new();
    for image in resources.iter() {
        if !store.insert_encoded(ImageKey::Src(image.src.clone()), &image.bytes) {
            log::warn!(
                "image {} could not be decoded; it shows as broken",
                image.url
            );
        }
    }
    log::debug!(
        "decoded {} of {} images for {}",
        store.len(),
        resources.len(),
        page.url
    );
    let store = Arc::new(store);
    *cached = Some((Arc::clone(resources), Arc::clone(&store)));
    store
}

/// Adds every outermost inline `<svg>` element of `document` to `images`,
/// keyed by its node, serialized from the parsed tree.
fn add_inline_svgs(document: &html::HTMLDocument, images: &mut ImageStore) {
    let is_svg_element = |id: NodeID| {
        matches!(
            document.node(id).map(|node| node.get_kind()),
            Some(html::NodeKind::Element(element))
                if *element.get_namespace() == html::Namespace::Svg
        )
    };
    for index in 0..document.nodes.len() {
        let id = NodeID::new(index);
        let Some(node) = document.node(id) else {
            continue;
        };
        let html::NodeKind::Element(element) = node.get_kind() else {
            continue;
        };
        if element.get_name() != "svg"
            || *element.get_namespace() != html::Namespace::Svg
            || node.get_parent().is_some_and(is_svg_element)
        {
            continue;
        }
        let Some(markup) = document.outer_html(id) else {
            continue;
        };
        if !images.insert_svg(ImageKey::InlineSvg(id), &markup) {
            log::warn!("inline <svg> (node {id:?}) could not be parsed; it paints nothing");
        }
    }
}

/// The `@font-face` fonts of the page decoded last, kept like
/// [`DECODED_IMAGES`] so relayouts reuse them.
type DecodedFonts = (Arc<[browser::FontResource]>, Arc<FontCollection>);
static DECODED_FONTS: Mutex<Option<DecodedFonts>> = Mutex::new(None);

/// The fonts layout measures and the compositor draws `page` with: the
/// system page font (or the bitmap fallback) as the default, plus the page's
/// downloaded web fonts, parsed once per page.
fn page_fonts(page: &Page) -> FontCollection {
    let mut fonts = FontCollection::new(page_font().cloned());
    let resources = &page.resources.fonts;
    if resources.is_empty() {
        return fonts;
    }
    let mut cached = DECODED_FONTS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let faces = match cached.as_ref() {
        Some((list, faces)) if Arc::ptr_eq(list, resources) => Arc::clone(faces),
        _ => {
            let mut faces = FontCollection::new(None);
            for font in resources.iter() {
                if !faces.add_face(&font.family, font.weight, font.italic, &font.bytes) {
                    log::warn!(
                        "web font {} for {:?} could not be parsed; text falls back",
                        font.url,
                        font.family
                    );
                }
            }
            log::debug!(
                "parsed {} of {} web fonts for {}",
                faces.face_count(),
                resources.len(),
                page.url
            );
            let faces = Arc::new(faces);
            *cached = Some((Arc::clone(resources), Arc::clone(&faces)));
            faces
        }
    };
    fonts.extend_faces(&faces);
    fonts
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
    // With an outline default font, text is rasterized at device
    // resolution; the bitmap fallback renders one device pixel per CSS
    // pixel and [`render_page`] scales the frame up.
    if layout.fonts.default_font().is_some() {
        let scale = scale_factor as f32;
        let scroll_y = (scroll_y * scale).round() / scale;
        let display_list = display_list(scroll_y);
        Compositor::new(width, height).compose_page(
            &display_list,
            &layout.fonts,
            &layout.images,
            scale,
        )
    } else {
        let display_list = display_list(scroll_y.round());
        Compositor::new(css_width, css_height).compose_page(
            &display_list,
            &layout.fonts,
            &layout.images,
            1.0,
        )
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
///
/// While CSS animations run, [`Self::animate`] restyles and lays out the
/// retained document again for a later moment of the document timeline.
pub(crate) struct PageLayout {
    /// The viewport width and height in CSS pixels.
    css_size: (u32, u32),
    tree: LayoutTree,
    /// Elements that can take focus from a click, in document order.
    focusable: Vec<NodeID>,
    /// Decoded `<img>` files and inline `<svg>` documents the tree's
    /// replaced boxes paint.
    images: ImageStore,
    /// The default and web fonts the tree was measured with.
    fonts: FontCollection,
    /// The parsed document and stylesheets, kept so animation frames only
    /// redo style and layout.
    document: html::HTMLDocument,
    stylesheets: Vec<css::Stylesheet>,
    /// The pointer and focus state the tree was styled with; animation frames
    /// reuse it.
    state: css::SelectorState,
    /// CSS animations that were delayed or playing when this layout was
    /// last styled; see [`css::ComputedStyles::running_animations`].
    running_animations: usize,
    /// How long each stage of building this layout took, until the app moves
    /// them to the Performance panel with [`Self::take_timeline`].
    timeline: Vec<TimelineEvent>,
}

impl PageLayout {
    /// Lays out `page` for a `width` × `height` CSS-pixel viewport at the
    /// start of its animation timeline.
    ///
    /// Times HTML parsing, each stylesheet's parse, image decoding (when the
    /// page has pictures), style computation, and layout as Performance
    /// events; see [`Self::take_timeline`]. Downloaded images are decoded once
    /// per page and shared by later layouts of it; inline `<svg>` elements are
    /// parsed for each layout, since their node ids come from this parse.
    #[cfg(test)]
    pub(crate) fn new(page: &Page, width: u32, height: u32) -> Self {
        Self::new_at(page, width, height, Duration::ZERO)
    }

    /// Lays out `page` like [`Self::new`], with CSS animations sampled
    /// `elapsed` after the page was first shown.
    #[cfg(test)]
    pub(crate) fn new_at(page: &Page, width: u32, height: u32, elapsed: Duration) -> Self {
        Self::new_with_state_at(page, width, height, css::SelectorState::default(), elapsed)
    }

    /// Lays out `page` like [`Self::new`], styled with the pointer and focus
    /// `state` and with CSS animations sampled `elapsed` after the page was
    /// first shown.
    #[cfg(test)]
    pub(crate) fn new_with_state_at(
        page: &Page,
        width: u32,
        height: u32,
        state: css::SelectorState,
        elapsed: Duration,
    ) -> Self {
        Self::new_with_document_state_at(page, None, width, height, state, elapsed)
    }

    /// Lays out `page` styled with the pointer and focus `state`, with CSS
    /// animations sampled `elapsed` after the page was first shown, from
    /// `live` instead of `page.html` when given.
    ///
    /// `live` is the page session's DOM after scripts changed it (see
    /// [`PageEvents::watch_dom`]). Its node IDs are the session's own, so hit
    /// tests on the result can be sent back to [`PageEvents::click`]. The
    /// page's stylesheets, downloaded images and fonts still come from `page`:
    /// a script that adds a stylesheet or changes an `<img src>` is not
    /// reflected, while inline `<svg>` changes are.
    pub(crate) fn new_with_document_state_at(
        page: &Page,
        live: Option<&html::HTMLDocument>,
        width: u32,
        height: u32,
        state: css::SelectorState,
        elapsed: Duration,
    ) -> Self {
        let mut timeline = Vec::new();
        let document = match live {
            Some(live) => live.clone(),
            None => {
                let (document, event) =
                    TimelineEvent::measure(EventCategory::Parse, "Parse HTML", || {
                        html::parse_raw_html(page.html.to_owned())
                    });
                timeline.push(event);
                document
            }
        };
        let focusable = document
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                let NodeKind::Element(element) = node.get_kind() else {
                    return None;
                };
                let name = element.get_name();
                let has_tabindex = element.get_attributes().iter().any(|attribute| {
                    attribute.get_name().eq_ignore_ascii_case("tabindex")
                        && attribute
                            .get_value()
                            .parse::<i32>()
                            .is_ok_and(|value| value >= 0)
                });
                let naturally_focusable =
                    matches!(name.as_str(), "button" | "input" | "select" | "textarea")
                        || matches!(name.as_str(), "a" | "area")
                            && element
                                .get_attributes()
                                .iter()
                                .any(|attribute| attribute.get_name().eq_ignore_ascii_case("href"));
                (has_tabindex || naturally_focusable).then(|| NodeID::new(index))
            })
            .collect();
        let stylesheets: Vec<_> = page
            .stylesheets
            .iter()
            .enumerate()
            .map(|(index, source)| {
                let label = format!("Parse stylesheet {}", index + 1);
                let (sheet, event) = TimelineEvent::measure(EventCategory::Parse, label, || {
                    css::parse_stylesheet(source)
                });
                timeline.push(event);
                sheet
            })
            .collect();
        let (images, event) = TimelineEvent::measure(EventCategory::Parse, "Decode images", || {
            let mut images = (*decoded_images(page)).clone();
            add_inline_svgs(&document, &mut images);
            images
        });
        // Pages without pictures leave the stage out of the panel.
        if !images.is_empty() {
            timeline.push(event);
        }
        let fonts = page_fonts(page);
        let (tree, running_animations, events) = style_and_layout(
            &document,
            &stylesheets,
            (&fonts, &images),
            (width, height),
            state,
            elapsed,
            "Recalculate style",
        );
        timeline.extend(events);
        Self {
            css_size: (width, height),
            tree,
            focusable,
            images,
            fonts,
            document,
            stylesheets,
            state,
            running_animations,
            timeline,
        }
    }

    /// Whether CSS animations can still change this layout, so the window
    /// should keep calling [`Self::animate`].
    pub(crate) fn is_animating(&self) -> bool {
        self.running_animations > 0
    }

    /// Restyles and lays out the page with CSS animations sampled `elapsed`
    /// after the page was first shown. The viewport size is unchanged.
    pub(crate) fn animate(&mut self, elapsed: Duration) {
        let (tree, running_animations, events) = style_and_layout(
            &self.document,
            &self.stylesheets,
            (&self.fonts, &self.images),
            self.css_size,
            self.state,
            elapsed,
            "Animation frame: recalculate style",
        );
        self.tree = tree;
        self.running_animations = running_animations;
        self.timeline.extend(events);
    }

    /// Removes and returns the timings recorded while building this layout.
    /// Later calls return an empty list.
    pub(crate) fn take_timeline(&mut self) -> Vec<TimelineEvent> {
        std::mem::take(&mut self.timeline)
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
            if let Some(rect) = text_range_rect(fragment, range.bytes, &self.fonts) {
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

    pub(crate) fn is_focusable(&self, node: NodeID) -> bool {
        self.focusable.contains(&node)
    }
}

/// Styles `document` with the pointer and focus `state` and CSS animations
/// sampled at `elapsed`, and lays it out for a `width` × `height` CSS-pixel viewport, measuring text with
/// `fonts` and sizing replaced elements with `images`.
///
/// Returns the layout, the number of running animations, and Performance
/// events for the style (named `style_label`) and layout stages.
fn style_and_layout(
    document: &html::HTMLDocument,
    stylesheets: &[css::Stylesheet],
    (fonts, images): (&FontCollection, &ImageStore),
    (width, height): (u32, u32),
    state: css::SelectorState,
    elapsed: Duration,
    style_label: &str,
) -> (LayoutTree, usize, [TimelineEvent; 2]) {
    let (styles, style_event) = TimelineEvent::measure(EventCategory::Style, style_label, || {
        css::style_document_with_state_at(document, stylesheets, state, elapsed)
    });
    let (tree, layout_event) = TimelineEvent::measure(
        EventCategory::Layout,
        format!("Layout {width}×{height}"),
        || {
            layout_document_with_images(
                document,
                &styles,
                Size {
                    width: width as f32,
                    height: height as f32,
                },
                fonts,
                images,
            )
        },
    );
    (tree, styles.running_animations, [style_event, layout_event])
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
    let x = fragment.rect.x + metrics.font_text_width(fragment.font, prefix, fragment.font_size);
    let width = metrics.font_text_width(fragment.font, matched, fragment.font_size);
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

/// Receives the page session's DOM each time a script task changes it.
///
/// Called on the session thread with a copy of the live tree, after the
/// page's scripts first run and after each click's listeners finish. See
/// [`PageEvents::watch_dom`].
pub(crate) type DomChanged = Arc<dyn Fn(html::HTMLDocument) + Send + Sync>;

enum PageCommand {
    Unload,
    Load {
        page: Page,
        console: Arc<dyn ConsoleSink>,
        alert_sink: Arc<dyn AlertSink>,
        runtime: BrowserRuntime,
        device_pixel_ratio: f64,
        viewport: Option<ViewportSize>,
        navigate: LocationNavigate,
        performance: Option<Arc<Mutex<PerformanceLog>>>,
    },
    Resize(ViewportSize),
    ScrollY(f64),
    DevicePixelRatio(f64),
    Click(Option<NodeID>),
    WatchDom(DomChanged),
    Snapshot(mpsc::Sender<String>),
    /// Answers once every earlier command has been handled.
    Flush(mpsc::Sender<()>),
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
    /// Drops the old document session after navigation; switching tabs never unloads it.
    pub(crate) fn unload(&self) {
        if self.commands.send(PageCommand::Unload).is_err() {
            log::debug!("page session already stopped");
        }
    }

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

    /// Returns a function that waits, up to `timeout`, until the session has
    /// handled every command sent before this call (such as a page load and
    /// its scripts), then returns the error that stopped the current page's
    /// scripts, or `None` when they finished (or have not run yet).
    ///
    /// Call the function off the UI thread: a script waiting on `alert`
    /// holds up the session until the UI thread shows the dialog.
    pub(crate) fn settled_script_error(
        &self,
        timeout: std::time::Duration,
    ) -> impl FnOnce() -> Option<String> + Send + 'static {
        let (sender, receiver) = mpsc::channel();
        let queued = self.commands.send(PageCommand::Flush(sender)).is_ok();
        let slot = Arc::clone(&self.script_error);
        move || {
            if queued && receiver.recv_timeout(timeout).is_err() {
                log::debug!("page session still busy; reading its script error early");
            }
            slot.lock().map_or(None, |error| error.clone())
        }
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
            None,
        );
    }

    /// Replaces the live document and exposes `device_pixel_ratio` and
    /// `viewport` (CSS pixels) to page scripts. `Location` navigation
    /// requests from the page are passed to `navigate` on the session thread.
    /// When `performance` is given, script evaluation and click listeners
    /// for this page are recorded there as [`EventCategory::Script`] events.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn load_with_viewport(
        &self,
        page: &Page,
        console: Arc<dyn ConsoleSink>,
        alert_sink: Arc<dyn AlertSink>,
        runtime: BrowserRuntime,
        device_pixel_ratio: f64,
        viewport: Option<ViewportSize>,
        navigate: LocationNavigate,
        performance: Option<Arc<Mutex<PerformanceLog>>>,
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
                performance,
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

    /// Reports DOM changes of the next loaded page to `changed`.
    ///
    /// Call this before [`Self::load_with_viewport`]; it applies to that load
    /// and lasts until [`Self::unload`]. Script changes are checked after the
    /// page's scripts run and after every click, so the shell can lay out
    /// the live tree. Timers never run on the session thread, so changes from
    /// `setTimeout` or `setInterval` callbacks are not reported.
    pub(crate) fn watch_dom(&self, changed: DomChanged) {
        if self.commands.send(PageCommand::WatchDom(changed)).is_err() {
            log::warn!("page session is not running; dropped DOM watcher");
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

/// Sends the session's DOM to `watcher` when it differs from `shown`, the
/// tree the shell last laid out, and remembers it as shown.
fn report_dom_change(
    session: &browser::PageSession,
    shown: &mut Option<html::HTMLDocument>,
    watcher: Option<&DomChanged>,
) {
    let Some(watcher) = watcher else {
        return;
    };
    let live = session.live_document();
    if shown.as_ref() == Some(&live) {
        return;
    }
    log::debug!("page script changed the DOM; relaying it for layout");
    *shown = Some(live.clone());
    watcher(live);
}

fn run_page_events(commands: mpsc::Receiver<PageCommand>, script_error: &Mutex<Option<String>>) {
    let mut session = None;
    // The tree the shell lays out and who to tell when scripts change it.
    let mut shown: Option<html::HTMLDocument> = None;
    let mut watcher: Option<DomChanged> = None;
    let mut performance: Option<Arc<Mutex<PerformanceLog>>> = None;
    let record = |log: &Option<Arc<Mutex<PerformanceLog>>>, event: TimelineEvent| {
        if let Some(log) = log
            && let Ok(mut log) = log.lock()
        {
            log.record(event);
        }
    };
    while let Ok(command) = commands.recv() {
        match command {
            PageCommand::Unload => {
                session = None;
                shown = None;
                watcher = None;
                performance = None;
                set_script_error(script_error, None);
            }
            PageCommand::Load {
                page,
                console,
                alert_sink,
                runtime,
                device_pixel_ratio,
                viewport,
                navigate,
                performance: log,
            } => {
                log::debug!("page session loading {}", page.url);
                performance = log;
                // A previous page's failure must not be blamed on this one.
                set_script_error(script_error, None);
                let controller = browser::request_controller(None);
                let (result, event) =
                    TimelineEvent::measure(EventCategory::Script, "Evaluate scripts", || {
                        runtime.open_page_with_viewport_and_navigation(
                            &page,
                            console,
                            alert_sink,
                            controller,
                            browser::security_context_for_url(&page.url),
                            device_pixel_ratio,
                            viewport,
                            navigate,
                        )
                    });
                record(&performance, event);
                match result {
                    Ok(loaded) => {
                        shown = Some(html::parse_raw_html(page.html.clone()));
                        report_dom_change(&loaded, &mut shown, watcher.as_ref());
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
                let (result, event) = TimelineEvent::measure(
                    EventCategory::Script,
                    "Click listeners",
                    || match target {
                        Some(node) => session.dispatch_click_at(node),
                        None => session.dispatch_click(),
                    },
                );
                record(&performance, event);
                if let Err(error) = result {
                    log::warn!("page click failed: {error}");
                }
                report_dom_change(session, &mut shown, watcher.as_ref());
            }
            PageCommand::WatchDom(changed) => watcher = Some(changed),
            PageCommand::Flush(sender) => {
                let _ = sender.send(());
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

    /// The color the welcome stylesheet gives `property` in its rule for
    /// `selector`, as RGBA bytes.
    fn welcome_color(page: &Page, selector: &str, property: &str) -> [u8; 4] {
        let sheet = css::parse_stylesheet(&page.stylesheets[0]);
        let value = sheet
            .rules
            .iter()
            .filter(|rule| rule.selector.0.trim() == selector)
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
    fn unloading_a_replaced_document_removes_old_click_listeners() {
        struct Recording(std::sync::mpsc::Sender<String>);
        impl AlertSink for Recording {
            fn alert(&self, message: &str) {
                let _ = self.0.send(message.into());
            }
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let alerts = Arc::new(Recording(tx));
        let events = PageEvents::new();
        let old = Page {
            url: "https://old.test/".into(),
            html: "<script></script>".into(),
            stylesheets: vec![],
            scripts: vec![
                "function click() { alert('old') } document.addEventListener('click', click)"
                    .into(),
            ],
            resources: Default::default(),
        };
        events.load(&old, alerts.clone(), test_runtime(), 1.0);
        events.click(None);
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),
            "old"
        );
        events.unload();
        events.click(None);
        let new = Page {
            url: "https://new.test/".into(),
            html: "<script></script>".into(),
            stylesheets: vec![],
            scripts: vec!["alert('new')".into()],
            resources: Default::default(),
        };
        events.load(&new, alerts, test_runtime(), 1.0);
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),
            "new"
        );
        assert!(rx.try_recv().is_err());
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
        let tree = layout::layout_document(
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
        let tree = layout::layout_document(
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
            resources: Default::default(),
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
            stylesheets: vec![".gone {display:none}".into()], scripts: vec![], resources: Default::default() };
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
        // The top bar spans the page from its corner, with the brand text
        // painted in it, and the white hero follows below.
        let bar = welcome_color(&page, ".topbar", "background-color");
        assert_eq!(&frame.pixels[..4], &bar);
        let inside_bar = (10 * 160 + 10) * 4;
        assert_eq!(&frame.pixels[inside_bar..inside_bar + 4], &bar);
        assert!(
            frame.pixels[..160 * 30 * 4]
                .as_chunks::<4>()
                .0
                .iter()
                .copied()
                .any(|p| p == welcome_color(&page, ".topbar .brand", "color"))
        );
        let in_hero = 160 * 70 * 4;
        assert_eq!(
            &frame.pixels[in_hero..in_hero + 4],
            &welcome_color(&page, ".hero", "background-color")
        );
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
            let bar = welcome_color(&page_source, ".topbar", "background-color");
            assert_eq!(&data[..4], &bar);
            let inside_bar =
                ((10.0 * scale) as usize * width as usize + (10.0 * scale) as usize) * 4;
            assert_eq!(&data[inside_bar..inside_bar + 4], &bar);
            assert!(
                data[..(30.0 * scale) as usize * width as usize * 4]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .copied()
                    .any(|p| p == welcome_color(&page_source, ".topbar .brand", "color"))
            );
            let in_hero = ((70.0 * scale) as u32 * width) as usize * 4;
            assert_eq!(
                &data[in_hero..in_hero + 4],
                &welcome_color(&page_source, ".hero", "background-color")
            );
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
            resources: Default::default(),
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
            None,
        );

        let (url, behavior) = navigate_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("Location.assign should reach the navigation callback");
        assert_eq!(url.as_str(), "https://example.test/next");
        assert_eq!(behavior, webapis::NavigationBehavior::Auto);
    }

    #[test]
    fn script_evaluation_is_recorded_on_the_performance_timeline() {
        let events = PageEvents::new();
        let performance = Arc::new(Mutex::new(PerformanceLog::new()));
        let page = Page {
            url: "https://example.test/".into(),
            html: "<script>1 + 1</script>".into(),
            stylesheets: Vec::new(),
            scripts: vec!["1 + 1".into()],
            resources: Default::default(),
        };
        events.load_with_viewport(
            &page,
            Arc::new(NoopConsole),
            test_alert_sink(),
            test_runtime(),
            1.0,
            None,
            Arc::new(|_, _| {}),
            Some(Arc::clone(&performance)),
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let labels = loop {
            let labels: Vec<_> = performance
                .lock()
                .unwrap()
                .entries()
                .iter()
                .map(|event| (event.category, event.label.clone()))
                .collect();
            if !labels.is_empty() || std::time::Instant::now() > deadline {
                break labels;
            }
            thread::sleep(std::time::Duration::from_millis(10));
        };
        assert_eq!(
            labels,
            [(EventCategory::Script, "Evaluate scripts".to_owned())]
        );
    }

    #[test]
    fn page_layout_times_each_stage_once() {
        let page = Page {
            url: "https://example.test/".into(),
            html: "<p>Hello</p>".into(),
            stylesheets: vec!["p { color: red }".into(), "p { margin: 0 }".into()],
            scripts: Vec::new(),
            resources: Default::default(),
        };
        let mut layout = PageLayout::new(&page, 320, 200);
        let labels: Vec<_> = layout
            .take_timeline()
            .into_iter()
            .map(|event| (event.category, event.label))
            .collect();
        assert_eq!(
            labels,
            [
                (EventCategory::Parse, "Parse HTML".to_owned()),
                (EventCategory::Parse, "Parse stylesheet 1".to_owned()),
                (EventCategory::Parse, "Parse stylesheet 2".to_owned()),
                (EventCategory::Style, "Recalculate style".to_owned()),
                (EventCategory::Layout, "Layout 320×200".to_owned()),
            ]
        );
        assert!(layout.take_timeline().is_empty());
    }

    #[test]
    fn css_animations_relayout_the_page_until_they_finish() {
        let page = Page {
            url: "https://example.test/".into(),
            html: "<div id='tall'></div>".into(),
            stylesheets: vec![
                "body { margin: 0 } \
                 @keyframes grow { from { height: 0 } to { height: 1000px } } \
                 #tall { animation: grow 1s linear forwards }"
                    .into(),
            ],
            scripts: Vec::new(),
            resources: Default::default(),
        };
        let mut layout = PageLayout::new(&page, 100, 200);
        assert!(layout.is_animating());
        assert_eq!(layout.max_scroll(), 0.0);

        layout.take_timeline();
        layout.animate(Duration::from_millis(500));
        assert_eq!(layout.max_scroll(), 300.0);
        assert!(layout.is_animating());
        let labels: Vec<_> = layout
            .take_timeline()
            .into_iter()
            .map(|event| event.label)
            .collect();
        assert_eq!(
            labels,
            ["Animation frame: recalculate style", "Layout 100×200"]
        );

        // A layout rebuilt mid-animation (after a resize) keeps the timeline.
        let resized = PageLayout::new_at(&page, 100, 100, Duration::from_millis(500));
        assert_eq!(resized.max_scroll(), 400.0);

        layout.animate(Duration::from_secs(2));
        assert_eq!(layout.max_scroll(), 800.0);
        assert!(!layout.is_animating());
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
                resources: Default::default(),
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
    fn clicking_the_welcome_oak_toggle_relays_the_changed_dom() {
        let page = browser::welcome_page();
        let (width, height) = (1280, 800);
        let layout = PageLayout::new(&page, width, height);
        let parsed = html::parse_raw_html(page.html.clone());
        let blue = parsed
            .get_element_by_id("pick-blue")
            .expect("welcome page has #pick-blue");
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
        // Find a point in the top bar that hits the toggle or its text.
        let on_blue = |node: NodeID| {
            node == blue || parsed.node(node).and_then(|node| node.get_parent()) == Some(blue)
        };
        let target = (0..60)
            .flat_map(|y| (0..width).step_by(4).map(move |x| (x as f32, y as f32)))
            .find_map(|(x, y)| layout.hit_test(x, y, 0.0).filter(|node| on_blue(*node)));
        assert!(
            target.is_some(),
            "#pick-blue should be hit-testable in the top bar"
        );

        let (changed_tx, changed_rx) = mpsc::channel();
        let events = PageEvents::new();
        events.watch_dom(Arc::new(move |dom| {
            let _ = changed_tx.send(dom);
        }));
        events.load(&page, test_alert_sink(), test_runtime(), 1.0);
        events.click(target);

        let dom = changed_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the toggle click should change the DOM");
        let root = dom.get_element_by_id("page").expect("#page");
        let class = match dom.node(root).map(|node| node.get_kind()) {
            Some(NodeKind::Element(element)) => element
                .get_attributes()
                .iter()
                .find(|attribute| attribute.get_name() == "class")
                .map(|attribute| attribute.get_value().clone()),
            _ => None,
        };
        assert_eq!(class.as_deref(), Some("page tree-blue"));

        // Laying out the relayed DOM shows the blue oak instead of the green.
        let blue_oak = layout_with_live(&page, &dom, width, height);
        let oak = |document: &html::HTMLDocument, class: &str| {
            document
                .nodes
                .iter()
                .position(|node| {
                    matches!(node.get_kind(), NodeKind::Element(element)
                    if element.get_name() == "svg"
                        && element.get_attributes().iter().any(|attribute| {
                            attribute.get_name() == "class"
                                && attribute.get_value().split(' ').any(|token| token == class)
                        }))
                })
                .map(NodeID::new)
                .expect("oak svg")
        };
        assert!(find(&layout.tree.root, oak(&dom, "oak-green")).is_some());
        assert!(find(&layout.tree.root, oak(&dom, "oak-blue")).is_none());
        assert!(find(&blue_oak.tree.root, oak(&dom, "oak-blue")).is_some());
        assert!(find(&blue_oak.tree.root, oak(&dom, "oak-green")).is_none());
    }

    fn layout_with_live(
        page: &Page,
        live: &html::HTMLDocument,
        width: u32,
        height: u32,
    ) -> PageLayout {
        PageLayout::new_with_document_state_at(
            page,
            Some(live),
            width,
            height,
            css::SelectorState::default(),
            Duration::ZERO,
        )
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

    #[test]
    fn img_files_and_inline_svg_are_laid_out_and_painted() {
        let red_square = r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20">
            <rect width="20" height="20" fill="red"/></svg>"#;
        let page = Page {
            url: "https://example.test/".into(),
            html: "<body><img src=\"square.svg\"><img src=\"broken.png\" alt=\"\">\
                   <svg width=\"10\" height=\"10\" viewBox=\"0 0 1 1\">\
                   <rect width=\"1\" height=\"1\" fill=\"blue\"></rect></svg></body>"
                .into(),
            stylesheets: vec!["body { margin: 0 } img, svg { display: block }".into()],
            scripts: Vec::new(),
            resources: browser::PageResources {
                images: vec![
                    browser::ImageResource {
                        src: "square.svg".into(),
                        url: "https://example.test/square.svg".into(),
                        bytes: red_square.as_bytes().into(),
                    },
                    browser::ImageResource {
                        src: "broken.png".into(),
                        url: "https://example.test/broken.png".into(),
                        bytes: b"not an image"[..].into(),
                    },
                ]
                .into(),
                ..Default::default()
            },
        };
        let mut layout = PageLayout::new(&page, 40, 40);
        assert!(
            layout
                .take_timeline()
                .iter()
                .any(|event| event.label == "Decode images")
        );
        // A second layout of the same page reuses the decoded pictures.
        assert!(Arc::ptr_eq(&decoded_images(&page), &decoded_images(&page)));
        let frame = compose_frame(&layout, 40, 40, 1.0, 0.0);
        let pixel = |x: u32, y: u32| {
            let offset = ((y * frame.width + x) * 4) as usize;
            frame.pixels[offset..offset + 4].to_vec()
        };
        // The 20px image, then the broken image (no height), then the
        // 10px inline SVG directly below.
        assert_eq!(pixel(10, 10), [255, 0, 0, 255]);
        assert_eq!(pixel(5, 25), [0, 0, 255, 255]);
        assert_eq!(pixel(15, 25), [255, 255, 255, 255]);
        assert_eq!(pixel(30, 10), [255, 255, 255, 255]);
    }

    #[test]
    fn web_fonts_from_font_face_are_used_for_matching_text() {
        let Some(data) = [
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            "C:/Windows/Fonts/arial.ttf",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
        ]
        .into_iter()
        .find_map(|path| std::fs::read(path).ok()) else {
            return;
        };
        let font = |family: &str, bytes: &[u8]| browser::FontResource {
            family: family.into(),
            weight: (400, 400),
            italic: false,
            url: format!("https://example.test/{family}.ttf"),
            bytes: bytes.into(),
        };
        let page = Page {
            url: "https://example.test/".into(),
            html: "<body><p id=\"web\">Web font</p><p id=\"bad\">Broken</p></body>".into(),
            stylesheets: vec![
                "body { margin: 0 } #web { font-family: 'Web' } #bad { font-family: Bad }".into(),
            ],
            scripts: Vec::new(),
            resources: browser::PageResources {
                fonts: vec![font("Web", &data), font("Bad", b"not a font")].into(),
                ..Default::default()
            },
        };
        let layout = PageLayout::new(&page, 200, 100);
        let fonts: Vec<_> = layout
            .find_fragments()
            .into_iter()
            .map(|(text, _, _)| (text.text.clone(), text.font))
            .collect();
        assert_eq!(
            fonts,
            [
                ("Web font".to_owned(), layout::FontId(1)),
                ("Broken".to_owned(), layout::FontId::DEFAULT),
            ],
            "the unparsable font is skipped, so its text uses the default"
        );
        let frame = compose_frame(&layout, 200, 100, 1.0, 0.0);
        assert!(
            frame
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[0] < 128)
        );
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
            resources: Default::default(),
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
            resources: Default::default(),
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
            resources: Default::default(),
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
            resources: Default::default(),
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
        let inside_bar = (10 * 160 + 10) * 4;
        assert_eq!(
            &frame.pixels[inside_bar..inside_bar + 4],
            &welcome_color(&page, ".topbar", "background-color")
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
            resources: Default::default(),
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
            resources: Default::default(),
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
    #[test]
    fn find_highlights_use_each_fragments_web_font_after_animation_and_reflow() {
        let Some(bytes) = [
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            "C:/Windows/Fonts/arial.ttf",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
        ]
        .into_iter()
        .find_map(|path| std::fs::read(path).ok()) else {
            return;
        };
        let page = Page {
            url: "https://font.test/".into(),
            html: "<body><div id='spacer'></div><p><span class='web'>Wi</span><span>Wi</span></p></body>".into(),
            stylesheets: vec!["body, p { margin: 0 } .web { font-family: Web } @keyframes grow { from { height: 0px } to { height: 40px } } #spacer { animation: grow 1s linear forwards }".into()],
            scripts: vec![],
            resources: browser::PageResources {
                fonts: vec![browser::FontResource { family: "Web".into(), weight: (400,400), italic: false,
                    url: "https://font.test/web.ttf".into(), bytes: bytes.into() }].into(),
                ..Default::default()
            },
        };
        let mut find = crate::find_in_page::FindInPage::default();
        find.insert("iW");
        for (width, elapsed) in [(320, Duration::ZERO), (180, Duration::from_millis(500))] {
            let mut layout = PageLayout::new_at(&page, width, 100, elapsed);
            layout.animate(elapsed);
            find.replace_runs(layout.find_text());
            assert_eq!(find.count(), "1/1");
            let fragments = layout.find_fragments();
            assert_eq!(fragments[0].0.font, layout::FontId(1));
            assert_eq!(fragments[1].0.font, layout::FontId::DEFAULT);
            let ranges = find.highlight_ranges(false);
            assert_eq!(ranges.len(), 2);
            let backgrounds = layout.text_backgrounds(Some(&find), false);
            for (range, _) in ranges {
                let fragment = fragments[range.run].0;
                let rect = backgrounds[&(fragment as *const layout::TextFragment)][0].rect;
                assert_eq!(
                    rect.x,
                    fragment.rect.x
                        + layout.fonts.font_text_width(
                            fragment.font,
                            &fragment.text[..range.bytes.start],
                            fragment.font_size
                        )
                );
                assert_eq!(
                    rect.width,
                    layout.fonts.font_text_width(
                        fragment.font,
                        &fragment.text[range.bytes],
                        fragment.font_size
                    )
                );
                assert_eq!(rect.y, fragment.rect.y);
                if elapsed > Duration::ZERO {
                    assert!(rect.y >= 20.0);
                }
            }
            for scale in [1.0, 1.5, 2.0] {
                assert!(
                    render_page_with_find(
                        &layout,
                        (width as f64 * scale) as u32,
                        (100.0 * scale) as u32,
                        scale,
                        0.0,
                        Some(&find),
                        false
                    )
                    .is_some()
                );
            }
        }
    }

    /// A page whose `#box` switches its `class` from `off` to `on` when clicked.
    fn toggling_page() -> Page {
        Page {
            url: "https://example.test/".into(),
            html: "<div id=\"box\" class=\"off\">Hi</div>".into(),
            stylesheets: vec![".on { display: none; }".into()],
            scripts: vec![
                "const box = document.getElementById('box');\n\
                 box.addEventListener('click', () => { box.setAttribute('class', 'on'); });"
                    .into(),
            ],
            resources: Default::default(),
        }
    }

    fn box_class(document: &html::HTMLDocument) -> Option<String> {
        let node = document.get_element_by_id("box")?;
        match document.node(node)?.get_kind() {
            NodeKind::Element(element) => element
                .get_attributes()
                .iter()
                .find(|attribute| attribute.get_name() == "class")
                .map(|attribute| attribute.get_value().clone()),
            _ => None,
        }
    }

    #[test]
    fn click_listener_dom_changes_reach_the_dom_watcher_once() {
        let (changed_tx, changed_rx) = mpsc::channel();
        let events = PageEvents::new();
        let page = toggling_page();
        events.watch_dom(Arc::new(move |dom| {
            let _ = changed_tx.send(dom);
        }));
        events.load_with_viewport(
            &page,
            Arc::new(NoopConsole),
            test_alert_sink(),
            test_runtime(),
            1.0,
            None,
            Arc::new(|_, _| {}),
            None,
        );
        // Answering a snapshot means every earlier command was handled.
        events.snapshot_html().expect("session should answer");
        assert!(
            changed_rx.try_recv().is_err(),
            "loading without script changes reports nothing"
        );

        let target = html::parse_raw_html(page.html.clone())
            .get_element_by_id("box")
            .expect("#box");
        events.click(Some(target));
        let dom = changed_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the click's DOM change should be reported");
        assert_eq!(box_class(&dom).as_deref(), Some("on"));

        // The class is already `on`, so a second click changes nothing.
        events.click(Some(target));
        events.snapshot_html().expect("session should answer");
        assert!(changed_rx.try_recv().is_err());
    }

    #[test]
    fn layout_uses_the_live_document_when_given() {
        let page = toggling_page();
        let parsed = html::parse_raw_html(page.html.clone());
        let target = parsed.get_element_by_id("box").expect("#box");
        let state = css::SelectorState::default();
        let original =
            PageLayout::new_with_document_state_at(&page, None, 200, 100, state, Duration::ZERO);
        assert!(original.hit_test(10.0, 12.0, 0.0).is_some());

        let mut live = parsed.clone();
        live.set_attribute(target, "class", "on")
            .expect("set class");
        let changed = PageLayout::new_with_document_state_at(
            &page,
            Some(&live),
            200,
            100,
            state,
            Duration::ZERO,
        );
        // `.on { display: none }` removed the only box from the live layout.
        assert_eq!(changed.hit_test(10.0, 12.0, 0.0), None);
    }
}
