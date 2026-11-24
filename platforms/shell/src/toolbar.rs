//! Browser chrome for the native shell: one unified toolbar row (navigation,
//! address field, actions) above a rounded page card.
//!
//! Window controls come from the operating system: each shell keeps its native
//! decorations. Where those overlap the window contents (macOS's traffic
//! lights on a transparent titlebar) they share the tab strip's row, which
//! reserves room for them; this toolbar sits below that row, so its Back
//! button lines up with the traffic lights' leading edge. Navigation and
//! address editing are handled by the shared shell.

use ab_glyph::{Font, FontVec, PxScale, ScaleFont, point};
use tiny_skia::{
    Color, FillRule, GradientStop, LinearGradient, Paint, Path, PathBuilder, Pixmap, Point, Rect,
    SpreadMode, Stroke, Transform,
};

/// Address toolbar height in logical pixels, below the shared tab strip.
pub(crate) const TOOLBAR_HEIGHT: f64 = 52.0;
/// Page origin below tabs and the address toolbar, in logical pixels.
pub(crate) const PAGE_TOP: f64 = TOOLBAR_HEIGHT + crate::tab_strip::HEIGHT;
/// Logical height of the strip holding every toolbar control. It stops above
/// the page card's shadow so the strip can be repainted on its own.
const CONTROLS_HEIGHT: f64 = crate::tab_strip::HEIGHT + 46.0;
/// Gap in logical pixels between the page card and the window's side and
/// bottom edges.
const PAGE_INSET: f64 = 8.0;
/// Corner radius in logical pixels of the page card and overlays docked to it.
pub(crate) const PAGE_RADIUS: f32 = 10.0;
const CENTER_Y: f32 = crate::tab_strip::HEIGHT as f32 + 26.0;
const BUTTON_SIZE: f32 = 28.0;
const BUTTON_TOP: f32 = CENTER_Y - BUTTON_SIZE / 2.0;
const ADDRESS_HEIGHT: f32 = 34.0;
const ADDRESS_MAX_WIDTH: f32 = 640.0;
/// Logical distance from the address field's left edge to its text, leaving
/// room for the Site Information button.
const ADDRESS_TEXT_LEADING: f32 = 30.0;
/// Logical distance from the window's left edge to the Back button. The
/// chevron is then centered under macOS's close traffic light (x ≈ 20.5) in
/// the tab strip row above, so the two groups line up.
const NAV_LEADING: f32 = 7.0;
/// Logical horizontal distance between neighboring navigation buttons' left
/// edges.
const NAV_STRIDE: f32 = 32.0;
/// Logical width of the Back, Forward, and Reload group.
const NAV_WIDTH: f32 = 2.0 * NAV_STRIDE + BUTTON_SIZE;
/// Minimum logical gap between the Reload button and the address field.
const NAV_ADDRESS_GAP: f32 = 12.0;
/// Logical distance from the window's left edge to the earliest position of
/// the address field, just past the navigation group.
const ADDRESS_LEADING: f32 = NAV_LEADING + NAV_WIDTH + NAV_ADDRESS_GAP;
/// Narrower windows drop the menu button to keep the address
/// field usable.
const TRAILING_MIN_WIDTH: f32 = 360.0;
const TEXT_SIZE: f32 = 13.0;
/// Logical height of the address-field load bar.
const LOADING_BAR_HEIGHT: f32 = 1.5;
/// Logical gap between the load bar and the address field's left and right edges.
const LOADING_BAR_INSET_X: f32 = 10.0;
/// Logical gap between the load bar and the address field's bottom edge.
const LOADING_BAR_INSET_BOTTOM: f32 = 2.0;
/// Where the solid accent gives way to the fade, as a fraction of the filled width.
const LOADING_BAR_FADE_START: f32 = 0.62;

/// A clickable region of the chrome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Control {
    /// Back navigation.
    Back,
    /// Forward navigation.
    Forward,
    /// Reload; fetches the current URL and repaints the loaded page.
    Reload,
    /// Opens information about the active document.
    SiteInfo,
    /// Address field; supports staged text editing.
    Address,
    /// Overflow menu placeholder.
    Menu,
}

const CONTROLS: [Control; 6] = [
    Control::Back,
    Control::Forward,
    Control::Reload,
    Control::Menu,
    Control::SiteInfo,
    Control::Address,
];

impl Control {
    /// Whether the control responds to hover and clicks in the current state.
    pub(crate) fn is_enabled(self, state: &ChromeState) -> bool {
        match self {
            Self::Back => state.can_go_back,
            Self::Forward => state.can_go_forward,
            _ => true,
        }
    }
}

/// Interaction and appearance state that changes how the chrome is painted.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ChromeState {
    /// Control under the pointer, if any.
    pub hovered: Option<Control>,
    /// Control with the left mouse button held down on it, if any.
    pub pressed: Option<Control>,
    /// Whether to use the dark palette.
    pub dark: bool,
    /// Whether the newest navigation or Refresh fetch is still pending.
    /// While set, a blue progress bar is drawn along the bottom of the
    /// address field.
    pub loading: bool,
    /// Fraction of the address-field load bar that is filled, from 0 to 1.
    /// The shell advances this from the moment a load starts. It is drawn
    /// only while [`ChromeState::loading`] is set.
    pub load_progress: f32,
    /// Whether a previous history entry is available.
    pub can_go_back: bool,
    /// Whether a forward history entry is available.
    pub can_go_forward: bool,
    /// Reserve space below the toolbar for Find in Page.
    pub find_open: bool,
    /// Whether the address field owns keyboard input.
    pub address_focused: bool,
    /// Fixed end of the address selection as a byte offset into the address
    /// text; the selection runs between this and
    /// [`ChromeState::address_caret`], which is the end that moves while
    /// dragging. `None`, or the same offset as the caret, means nothing is
    /// selected. A selection is drawn as a highlight instead of a caret and
    /// the next edit replaces it; meaningful only while
    /// [`ChromeState::address_focused`] is set.
    pub address_anchor: Option<usize>,
    /// Caret position as a byte offset into the address text, drawn while the
    /// field is focused and nothing is selected. Offsets past the end, or
    /// inside a multi-byte character, are drawn at the nearest earlier
    /// character boundary.
    pub address_caret: usize,
}

impl ChromeState {
    /// Returns the selected byte range of `address` as `(start, end)` with
    /// `start < end`, snapped to character boundaries, or `None` when nothing
    /// is selected.
    pub(crate) fn address_selection(&self, address: &str) -> Option<(usize, usize)> {
        let anchor = floor_char_boundary(address, self.address_anchor?);
        let caret = floor_char_boundary(address, self.address_caret);
        (anchor != caret).then(|| (anchor.min(caret), anchor.max(caret)))
    }
}

/// A rectangle in device pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PixelRect {
    /// Left edge in device pixels.
    pub x: u32,
    /// Top edge in device pixels.
    pub y: u32,
    /// Width in device pixels; always nonzero.
    pub width: u32,
    /// Height in device pixels; always nonzero.
    pub height: u32,
}

/// Returns the page card's area for a window of `width` × `height` device
/// pixels, or `None` when the window is too small to show any page.
pub(crate) fn page_rect(width: u32, height: u32, scale_factor: f64) -> Option<PixelRect> {
    let inset = (PAGE_INSET * scale_factor).round() as u32;
    let top = (PAGE_TOP * scale_factor).round() as u32;
    let page_width = width.checked_sub(inset * 2).filter(|w| *w > 0)?;
    let page_height = height.checked_sub(top + inset).filter(|h| *h > 0)?;
    Some(PixelRect {
        x: inset,
        y: top,
        width: page_width,
        height: page_height,
    })
}

/// Page viewport with the Find bar's row reserved, in device pixels.
pub(crate) fn page_rect_with_find(
    width: u32,
    height: u32,
    scale: f64,
    find: bool,
) -> Option<PixelRect> {
    let mut rect = page_rect(width, height, scale)?;
    if find {
        let extra = (crate::find_in_page::HEIGHT as f64 * scale).round() as u32;
        rect.y += extra;
        rect.height = rect.height.checked_sub(extra).filter(|h| *h > 0)?;
    }
    Some(rect)
}

/// Returns the enabled or disabled control at a point given in logical
/// pixels, for a window `width` logical pixels wide.
pub(crate) fn hit_test(x: f64, y: f64, width: f64) -> Option<Control> {
    let (x, y) = (x as f32, y as f32);
    CONTROLS.into_iter().find(|control| {
        control_rect(*control, width as f32).is_some_and(|rect| {
            x >= rect.left() && x < rect.right() && y >= rect.top() && y < rect.bottom()
        })
    })
}

/// Loads the first readable font file in `candidates` for toolbar text, or
/// `None` if none loads; the chrome then omits its text.
///
/// Each platform shell lists its system UI font files in order of preference.
/// Variable fonts with an optical-size axis are set to the toolbar text size.
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
pub(crate) fn load_ui_font<P: AsRef<std::path::Path>>(
    candidates: impl IntoIterator<Item = P>,
) -> Option<FontVec> {
    use ab_glyph::VariableFont;

    let font = candidates.into_iter().find_map(|path| {
        let mut font = FontVec::try_from_vec(std::fs::read(path.as_ref()).ok()?).ok()?;
        // SF and other variable UI fonts pick the optical size designed for
        // small text; fonts without an `opsz` axis ignore this.
        font.set_variation(b"opsz", TEXT_SIZE);
        log::debug!("font file chosen: {}", path.as_ref().display());
        Some(font)
    });
    if font.is_none() {
        log::warn!("no UI font could be loaded; text will not render");
    }
    font
}

/// Paints the whole window: frame, page card holding `page`, and toolbar.
///
/// `width` and `height` are in device pixels and `page` should match
/// [`page_rect`]'s size; a missing or mismatched page shows a blank card.
/// Returns `None` only for a zero-sized window.
pub(crate) fn compose(
    width: u32,
    height: u32,
    scale_factor: f64,
    state: &ChromeState,
    address: &str,
    page: Option<&Pixmap>,
    font: Option<&FontVec>,
) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(width, height)?;
    let palette = &if state.dark { dark() } else { light() };
    let scale = scale_factor as f32;
    fill_frame(&mut pixmap, height, palette);
    if let Some(card) = page_rect_with_find(width, height, scale_factor, state.find_open) {
        draw_page_card(&mut pixmap, card, scale, height, palette, page);
    }
    draw_toolbar(
        &mut pixmap,
        width as f32 / scale,
        scale,
        state,
        palette,
        font,
        address,
    );
    Some(pixmap)
}

/// Repaints only the window's top [`controls_height_in_pixels`] rows, which
/// hold every toolbar control but none of the page card or its shadow.
///
/// The result matches the same rows of [`compose`] for the same arguments, so
/// hover, press, and focus changes can skip repainting the page. `height` is
/// the full window height in device pixels.
pub(crate) fn compose_controls(
    width: u32,
    height: u32,
    scale_factor: f64,
    state: &ChromeState,
    address: &str,
    font: Option<&FontVec>,
) -> Option<Pixmap> {
    let strip = controls_height_in_pixels(scale_factor).min(height);
    let mut pixmap = Pixmap::new(width, strip)?;
    let palette = &if state.dark { dark() } else { light() };
    let scale = scale_factor as f32;
    fill_frame(&mut pixmap, height, palette);
    draw_toolbar(
        &mut pixmap,
        width as f32 / scale,
        scale,
        state,
        palette,
        font,
        address,
    );
    Some(pixmap)
}

/// Height in device pixels of the strip that [`compose_controls`] repaints.
pub(crate) fn controls_height_in_pixels(scale_factor: f64) -> u32 {
    (CONTROLS_HEIGHT * scale_factor).floor() as u32
}

/// Fills `pixmap` row by row with the frame's vertical gradient, as it appears
/// in a window `window_height` device pixels tall.
fn fill_frame(pixmap: &mut Pixmap, window_height: u32, palette: &Palette) {
    let width = pixmap.width() as usize;
    for (y, row) in pixmap.data_mut().chunks_exact_mut(width * 4).enumerate() {
        let color = frame_color(y as u32, window_height, palette);
        for pixel in row.as_chunks_mut::<4>().0.iter_mut() {
            pixel.copy_from_slice(&color);
        }
    }
}

fn frame_color(y: u32, window_height: u32, palette: &Palette) -> [u8; 4] {
    let t = y as f32 / window_height.saturating_sub(1).max(1) as f32;
    let (top, bottom) = (palette.frame_top, palette.frame_bottom);
    let mix = |a: f32, b: f32| ((a + (b - a) * t) * 255.0).round() as u8;
    [
        mix(top.red(), bottom.red()),
        mix(top.green(), bottom.green()),
        mix(top.blue(), bottom.blue()),
        255,
    ]
}

/// Draws the page card using only row copies and perimeter strokes, so its
/// cost stays proportional to the card's edge rather than its area.
fn draw_page_card(
    pixmap: &mut Pixmap,
    card: PixelRect,
    scale: f32,
    window_height: u32,
    palette: &Palette,
    page: Option<&Pixmap>,
) {
    let stride = pixmap.width() as usize * 4;
    let row_bytes = card.width as usize * 4;
    let page = page.filter(|page| page.width() == card.width && page.height() == card.height);
    let data = pixmap.data_mut();
    for row in 0..card.height as usize {
        let start = (card.y as usize + row) * stride + card.x as usize * 4;
        let target = &mut data[start..start + row_bytes];
        match page {
            Some(page) => target.copy_from_slice(&page.data()[row * row_bytes..][..row_bytes]),
            None => target.fill(255),
        }
    }

    let (x, y) = (card.x as f32, card.y as f32);
    let (w, h) = (card.width as f32, card.height as f32);
    let radius = (PAGE_RADIUS * scale).min(w / 2.0).min(h / 2.0);
    let identity = Transform::identity();
    // Round the corners by painting the frame back over each corner's
    // outside; anti-aliasing blends the curve into the page.
    let k = radius * 0.4477;
    for (cx, cy, dx, dy) in [
        (x, y, 1.0, 1.0),
        (x + w, y, -1.0, 1.0),
        (x, y + h, 1.0, -1.0),
        (x + w, y + h, -1.0, -1.0),
    ] {
        let mut builder = PathBuilder::new();
        builder.move_to(cx, cy);
        builder.line_to(cx + dx * radius, cy);
        builder.cubic_to(cx + dx * k, cy, cx, cy + dy * k, cx, cy + dy * radius);
        builder.close();
        let [r, g, b, _] = frame_color(cy as u32, window_height, palette);
        if let Some(path) = builder.finish() {
            fill(pixmap, &path, rgba(r, g, b, 255), identity);
        }
    }
    // tiny-skia has no blur, so fake a soft shadow with rings that fade
    // outward from the card's edge.
    for layer in 1..=4 {
        let spread = (layer as f32 - 0.5) * scale;
        let mut color = palette.shadow;
        color.set_alpha((color.alpha() * (5 - layer) as f32).min(1.0));
        if let Some(ring) = rounded_rect(
            x - spread,
            y - spread,
            w + spread * 2.0,
            h + spread * 2.0,
            radius + spread,
        ) {
            stroke(pixmap, &ring, color, scale, identity);
        }
    }
    if let Some(edge) = rounded_rect(x + 0.5, y + 0.5, w - 1.0, h - 1.0, radius - 0.5) {
        stroke(pixmap, &edge, palette.card_border, 1.0, identity);
    }
}

fn draw_toolbar(
    pixmap: &mut Pixmap,
    width: f32,
    scale: f32,
    state: &ChromeState,
    palette: &Palette,
    font: Option<&FontVec>,
    address: &str,
) {
    let transform = Transform::from_scale(scale, scale);
    for control in CONTROLS {
        if matches!(control, Control::Address | Control::SiteInfo) {
            continue;
        }
        let Some(rect) = control_rect(control, width) else {
            continue;
        };
        let active = control.is_enabled(state);
        let background = match (state.pressed, state.hovered) {
            (Some(pressed), _) if pressed == control && active => Some(palette.pressed),
            (_, Some(hovered)) if hovered == control && active => Some(palette.hover),
            _ => None,
        };
        if let Some(color) = background
            && let Some(path) = rounded_rect(rect.x(), rect.y(), rect.width(), rect.height(), 7.0)
        {
            fill(pixmap, &path, color, transform);
        }
        let color = if active {
            palette.icon
        } else {
            palette.icon_disabled
        };
        let (cx, cy) = (rect.x() + rect.width() / 2.0, CENTER_Y);
        draw_icon(pixmap, control, cx, cy, color, transform);
    }
    if let Some(rect) = control_rect(Control::Address, width) {
        draw_address_field(pixmap, rect, scale, state, palette, font, address);
    }
}

fn draw_icon(
    pixmap: &mut Pixmap,
    control: Control,
    cx: f32,
    cy: f32,
    color: Color,
    transform: Transform,
) {
    let mut builder = PathBuilder::new();
    match control {
        Control::Back | Control::Forward => {
            let direction = if control == Control::Back { 1.0 } else { -1.0 };
            builder.move_to(cx + 2.5 * direction, cy - 6.0);
            builder.line_to(cx - 3.5 * direction, cy);
            builder.line_to(cx + 2.5 * direction, cy + 6.0);
        }
        Control::Reload => {
            // A clockwise arc with its gap at the top, ending in an arrowhead.
            let radius = 6.5;
            let (start, sweep) = (-60f32.to_radians(), 290f32.to_radians());
            for step in 0..=32 {
                let angle = start + sweep * step as f32 / 32.0;
                let (x, y) = (cx + radius * angle.cos(), cy + radius * angle.sin());
                if step == 0 {
                    builder.move_to(x, y);
                } else {
                    builder.line_to(x, y);
                }
            }
            if let Some(path) = builder.finish() {
                stroke(pixmap, &path, color, 1.6, transform);
            }
            let end = start + sweep;
            let (ex, ey) = (cx + radius * end.cos(), cy + radius * end.sin());
            let (tx, ty) = (-end.sin(), end.cos());
            let (nx, ny) = (end.cos(), end.sin());
            let mut head = PathBuilder::new();
            head.move_to(ex + tx * 3.2, ey + ty * 3.2);
            head.line_to(ex + nx * 3.0 - tx * 1.2, ey + ny * 3.0 - ty * 1.2);
            head.line_to(ex - nx * 3.0 - tx * 1.2, ey - ny * 3.0 - ty * 1.2);
            head.close();
            if let Some(path) = head.finish() {
                fill(pixmap, &path, color, transform);
            }
            return;
        }
        Control::Menu => {
            for offset in [-5.5, 0.0, 5.5] {
                builder.push_circle(cx + offset, cy, 1.6);
            }
            if let Some(path) = builder.finish() {
                fill(pixmap, &path, color, transform);
            }
            return;
        }
        _ => return,
    }
    if let Some(path) = builder.finish() {
        stroke(pixmap, &path, color, 1.7, transform);
    }
}

fn draw_address_field(
    pixmap: &mut Pixmap,
    rect: Rect,
    scale: f32,
    state: &ChromeState,
    palette: &Palette,
    font: Option<&FontVec>,
    address: &str,
) {
    let transform = Transform::from_scale(scale, scale);
    let hovered = state.hovered == Some(Control::Address);
    let Some(shape) = rounded_rect(rect.x(), rect.y(), rect.width(), rect.height(), 10.0) else {
        return;
    };
    let field = if hovered {
        palette.field_hover
    } else {
        palette.field
    };
    fill(pixmap, &shape, field, transform);
    // A device-pixel hairline stays crisp at every scale factor.
    stroke(
        pixmap,
        &shape,
        if state.address_focused {
            palette.accent
        } else {
            palette.field_border
        },
        if state.address_focused {
            1.25 / scale
        } else {
            1.0 / scale
        },
        transform,
    );

    let cy = CENTER_Y;
    let button = site_info_button(rect);
    let cx = button.x() + button.width() / 2.0;
    let background = if state.pressed == Some(Control::SiteInfo) {
        palette.pressed
    } else if state.hovered == Some(Control::SiteInfo) {
        palette.hover
    } else if state.dark {
        palette.frame_top
    } else {
        palette.field_border
    };
    if let Some(path) = rounded_rect(
        button.x(),
        button.y(),
        button.width(),
        button.height(),
        11.0,
    ) {
        fill(pixmap, &path, background, transform);
    }
    let mut sliders = PathBuilder::new();
    for (dy, knob) in [(-3.0, -2.0), (3.0, 2.0)] {
        let y = cy + dy;
        sliders.move_to(cx - 5.0, y);
        sliders.line_to(cx + 5.0, y);
        sliders.move_to(cx + knob, y - 2.0);
        sliders.line_to(cx + knob, y + 2.0);
    }
    if let Some(path) = sliders.finish() {
        stroke(pixmap, &path, palette.icon, 1.4, transform);
    }

    let star_x = rect.right() - 18.0;
    let mut star = PathBuilder::new();
    for point in 0..10 {
        let radius = if point % 2 == 0 { 6.0 } else { 2.6 };
        let angle = (-90.0 + point as f32 * 36.0).to_radians();
        let (x, y) = (star_x + radius * angle.cos(), cy + radius * angle.sin());
        if point == 0 {
            star.move_to(x, y);
        } else {
            star.line_to(x, y);
        }
    }
    star.close();
    if let Some(path) = star.finish() {
        stroke(pixmap, &path, palette.text_muted, 1.3, transform);
    }

    let text_x = rect.x() + ADDRESS_TEXT_LEADING;
    let mut clip_x = star_x - 12.0;
    if state.loading
        && let Some(font) = font
    {
        // Reserve a separate text lane; URL, selection and caret stop before it.
        // At unusually narrow widths shrink the label rather than cover controls.
        let unit_width = text_advance(font, scale, "Loading...") / scale;
        let size = 11.0_f32.min((rect.width() - 74.0).max(1.0) / unit_width.max(1.0));
        let advance = text_advance(font, size * scale, "Loading...") / scale;
        let x = rect.right() - 30.0 - advance;
        clip_x = x - 6.0;
        let scaled = font.as_scaled(PxScale::from(size * scale));
        let baseline = cy * scale + (scaled.ascent() + scaled.descent()) / 2.0;
        draw_text(
            pixmap,
            font,
            size * scale,
            x * scale,
            baseline,
            (rect.right() - 28.0) * scale,
            palette.accent,
            "Loading...",
        );
    }
    let selection = state.address_selection(address);
    // Logical x of the caret, before the glyph at `address_caret`.
    let mut caret_x = text_x;
    if let Some(font) = font {
        let size = TEXT_SIZE * scale;
        let scaled = font.as_scaled(PxScale::from(size));
        let baseline = cy * scale + (scaled.ascent() + scaled.descent()) / 2.0;
        if state.address_focused
            && let Some((start, end)) = selection
        {
            let left = (text_x + text_advance(font, size, &address[..start]) / scale).min(clip_x);
            let right = (text_x + text_advance(font, size, &address[..end]) / scale).min(clip_x);
            if let Some(highlight) = Rect::from_xywh(left - 1.0, cy - 9.0, right - left + 2.0, 18.0)
            {
                fill(
                    pixmap,
                    &PathBuilder::from_rect(highlight),
                    palette.selection,
                    transform,
                );
            }
        }
        let caret = floor_char_boundary(address, state.address_caret);
        caret_x = text_x + text_advance(font, size, &address[..caret]) / scale;
        draw_text(
            pixmap,
            font,
            size,
            text_x * scale,
            baseline,
            clip_x * scale,
            if state.address_focused || state.dark {
                palette.text
            } else {
                palette.text_muted
            },
            address,
        );
    }
    draw_loading_bar(pixmap, rect, transform, state, palette);
    if state.address_focused && selection.is_none() {
        let x = caret_x.min(clip_x - 2.0) + 1.0;
        if let Some(caret) = Rect::from_xywh(x, cy - 8.0, 1.5, 16.0) {
            fill(
                pixmap,
                &PathBuilder::from_rect(caret),
                palette.text,
                transform,
            );
        }
    }
}

/// How much of the address-field load bar is filled after `elapsed`.
///
/// The curve starts with a visible segment and eases toward about 92% of
/// the field. It stays short of full so the bar is still moving when the
/// load completes and the bar is removed.
pub(crate) fn loading_bar_fraction(elapsed: std::time::Duration) -> f32 {
    let eased = 1.0 - (-elapsed.as_secs_f32() * 1.35).exp();
    (0.08 + eased * 0.84).min(0.92)
}

/// Draws the load indicator along the bottom inner edge of the address field.
///
/// The bar is a hairline. Its leading edge, the side the load is moving
/// toward, fades from the accent color to transparent.
fn draw_loading_bar(
    pixmap: &mut Pixmap,
    rect: Rect,
    transform: Transform,
    state: &ChromeState,
    palette: &Palette,
) {
    if !state.loading {
        return;
    }
    let height = LOADING_BAR_HEIGHT;
    let y = rect.bottom() - height - LOADING_BAR_INSET_BOTTOM;
    let x = rect.x() + LOADING_BAR_INSET_X;
    let track_w = (rect.width() - LOADING_BAR_INSET_X * 2.0).max(0.0);
    let progress = state.load_progress.clamp(0.0, 1.0);
    if track_w < 1.0 || progress <= 0.0 {
        return;
    }
    let fill_w = (track_w * progress).clamp(4.0_f32.min(track_w), track_w);
    let Some(path) = rounded_rect(x, y, fill_w, height, height / 2.0) else {
        return;
    };
    let mut faded = palette.accent;
    faded.set_alpha(0.0);
    let Some(shader) = LinearGradient::new(
        Point::from_xy(x, y),
        Point::from_xy(x + fill_w, y),
        vec![
            GradientStop::new(0.0, palette.accent),
            GradientStop::new(LOADING_BAR_FADE_START, palette.accent),
            GradientStop::new(1.0, faded),
        ],
        SpreadMode::Pad,
        Transform::identity(),
    ) else {
        fill(pixmap, &path, palette.accent, transform);
        return;
    };
    let paint = Paint {
        shader,
        ..Default::default()
    };
    pixmap.fill_path(&path, &paint, FillRule::Winding, transform, None);
}

/// Returns the byte offset in `address` of the character boundary nearest to
/// logical x coordinate `x`, for placing the caret where the pointer presses
/// or drags in the address field of a toolbar `width` logical pixels wide
/// `scale` is the device scale factor the field
/// is painted at. Points left of the text give 0 and points right of it give
/// the address length; without a font every point maps to 0.
pub(crate) fn address_offset_at(
    x: f64,
    width: f64,
    scale: f32,
    font: Option<&FontVec>,
    address: &str,
) -> usize {
    let (Some(font), Some(rect)) = (font, control_rect(Control::Address, width as f32)) else {
        return 0;
    };
    let size = TEXT_SIZE * scale;
    let target = x as f32 - (rect.x() + ADDRESS_TEXT_LEADING);
    address
        .char_indices()
        .map(|(index, _)| index)
        .chain([address.len()])
        .min_by(|a, b| {
            let distance = |index: &usize| {
                (text_advance(font, size, &address[..*index]) / scale - target).abs()
            };
            distance(a).total_cmp(&distance(b))
        })
        .unwrap_or(0)
}

/// Returns the largest character boundary of `text` at or before `index`.
pub(crate) fn floor_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// Returns the pen advance in device pixels of `text` set at `size`, matching
/// the position [`draw_text`] returns relative to its starting `x`.
pub(crate) fn text_advance(font: &FontVec, size: f32, text: &str) -> f32 {
    let scaled = font.as_scaled(PxScale::from(size));
    let mut advance = 0.0;
    let mut previous = None;
    for character in text.chars() {
        let id = scaled.glyph_id(character);
        if let Some(previous) = previous {
            advance += scaled.kern(previous, id);
        }
        previous = Some(id);
        advance += scaled.h_advance(id);
    }
    advance
}

/// Draws `text` with its baseline at `baseline` (device pixels), clipping
/// glyph pixels at `clip_x`. Returns the pen position after the last glyph.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_text(
    pixmap: &mut Pixmap,
    font: &FontVec,
    size: f32,
    x: f32,
    baseline: f32,
    clip_x: f32,
    color: Color,
    text: &str,
) -> f32 {
    let scaled = font.as_scaled(PxScale::from(size));
    let (width, height) = (pixmap.width() as i32, pixmap.height() as i32);
    let data = pixmap.data_mut();
    let mut caret = x;
    let mut previous = None;
    for character in text.chars() {
        let id = scaled.glyph_id(character);
        if let Some(previous) = previous {
            caret += scaled.kern(previous, id);
        }
        previous = Some(id);
        let glyph = id.with_scale_and_position(size, point(caret, baseline));
        caret += scaled.h_advance(id);
        let Some(outline) = font.outline_glyph(glyph) else {
            continue;
        };
        let bounds = outline.px_bounds();
        outline.draw(|gx, gy, coverage| {
            let px = bounds.min.x as i32 + gx as i32;
            let py = bounds.min.y as i32 + gy as i32;
            if px < 0 || py < 0 || px >= width || py >= height || px as f32 >= clip_x {
                return;
            }
            let alpha = coverage.clamp(0.0, 1.0) * color.alpha();
            let index = (py * width + px) as usize * 4;
            for (channel, source) in [color.red(), color.green(), color.blue(), 1.0]
                .into_iter()
                .enumerate()
            {
                let destination = f32::from(data[index + channel]);
                data[index + channel] =
                    (source * 255.0 * alpha + destination * (1.0 - alpha)).round() as u8;
            }
        });
    }
    caret
}

/// Returns where `control` sits in a toolbar `width` logical pixels wide, or
/// `None` when the window is too narrow to show it.
///
/// Back, Forward, and Reload stay pinned at [`NAV_LEADING`], under the
/// window's traffic lights, while the address field centers independently.
fn control_rect(control: Control, width: f32) -> Option<Rect> {
    let button = |x: f32| Rect::from_xywh(x, BUTTON_TOP, BUTTON_SIZE, BUTTON_SIZE);
    let trailing = width >= TRAILING_MIN_WIDTH;
    match control {
        Control::Back => button(NAV_LEADING),
        Control::Forward => button(NAV_LEADING + NAV_STRIDE),
        Control::Reload => button(NAV_LEADING + 2.0 * NAV_STRIDE),
        Control::Menu => button(width - 44.0).filter(|_| trailing),
        Control::Address => address_rect(width, trailing),
        Control::SiteInfo => address_rect(width, trailing).map(site_info_button),
    }
}

fn site_info_button(field: Rect) -> Rect {
    Rect::from_xywh(field.x() + 5.0, CENTER_Y - 11.0, 22.0, 22.0)
        .expect("positive Site Information button size")
}

/// Left edge of the Site Information button in logical window pixels.
pub(crate) fn site_info_anchor(width: f64) -> Option<f32> {
    control_rect(Control::SiteInfo, width as f32).map(|rect| rect.left())
}

/// Returns where the address field sits, or `None` when the window is too
/// narrow for it. `trailing` says whether the menu is shown.
fn address_rect(width: f32, trailing: bool) -> Option<Rect> {
    let leading = ADDRESS_LEADING;
    let right = if trailing { width - 92.0 } else { width - 12.0 };
    let available = right - leading;
    if available < 80.0 {
        return None;
    }
    let field_width = available.min(ADDRESS_MAX_WIDTH);
    // Center in the window like Safari, but never under other controls.
    let x = ((width - field_width) / 2.0).clamp(leading, right - field_width);
    Rect::from_xywh(
        x,
        CENTER_Y - ADDRESS_HEIGHT / 2.0,
        field_width,
        ADDRESS_HEIGHT,
    )
}

pub(crate) fn rounded_rect(x: f32, y: f32, w: f32, h: f32, radius: f32) -> Option<Path> {
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    // Cubic Bézier quarter-circle approximation: control points sit 0.4477·r
    // from the corner.
    let k = r * 0.4477;
    let mut builder = PathBuilder::new();
    builder.move_to(x + r, y);
    builder.line_to(x + w - r, y);
    builder.cubic_to(x + w - k, y, x + w, y + k, x + w, y + r);
    builder.line_to(x + w, y + h - r);
    builder.cubic_to(x + w, y + h - k, x + w - k, y + h, x + w - r, y + h);
    builder.line_to(x + r, y + h);
    builder.cubic_to(x + k, y + h, x, y + h - k, x, y + h - r);
    builder.line_to(x, y + r);
    builder.cubic_to(x, y + k, x + k, y, x + r, y);
    builder.close();
    builder.finish()
}

/// Fills `path` with a solid `color`.
pub(crate) fn fill(pixmap: &mut Pixmap, path: &Path, color: Color, transform: Transform) {
    let mut paint = Paint::default();
    paint.set_color(color);
    pixmap.fill_path(path, &paint, FillRule::Winding, transform, None);
}

/// Strokes `path` with round caps and joins, `width` in transformed units.
pub(crate) fn stroke(
    pixmap: &mut Pixmap,
    path: &Path,
    color: Color,
    width: f32,
    transform: Transform,
) {
    let mut paint = Paint::default();
    paint.set_color(color);
    let stroke = Stroke {
        width,
        line_cap: tiny_skia::LineCap::Round,
        line_join: tiny_skia::LineJoin::Round,
        ..Stroke::default()
    };
    pixmap.stroke_path(path, &paint, &stroke, transform, None);
}

pub(crate) fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color::from_rgba8(r, g, b, a)
}

/// Shared browser-chrome colors for one appearance.
pub(crate) struct Palette {
    pub(crate) frame_top: Color,
    pub(crate) frame_bottom: Color,
    pub(crate) shadow: Color,
    pub(crate) card_border: Color,
    pub(crate) icon: Color,
    pub(crate) icon_disabled: Color,
    pub(crate) hover: Color,
    pub(crate) pressed: Color,
    pub(crate) field: Color,
    pub(crate) field_hover: Color,
    pub(crate) field_border: Color,
    pub(crate) text: Color,
    pub(crate) text_muted: Color,
    pub(crate) accent: Color,
    pub(crate) on_accent: Color,
    pub(crate) active_tab: Color,
    pub(crate) panel: Color,
    pub(crate) panel_border: Color,
    pub(crate) selection: Color,
}

/// Returns the chrome palette for the dark or light appearance.
pub(crate) fn palette(dark: bool) -> Palette {
    if dark { self::dark() } else { light() }
}

fn light() -> Palette {
    Palette {
        frame_top: rgba(0xf4, 0xf5, 0xf8, 255),
        frame_bottom: rgba(0xeb, 0xed, 0xf1, 255),
        shadow: rgba(15, 23, 42, 7),
        card_border: rgba(15, 23, 42, 22),
        icon: rgba(0x2f, 0x35, 0x42, 255),
        icon_disabled: rgba(0x2f, 0x35, 0x42, 80),
        hover: rgba(15, 23, 42, 14),
        pressed: rgba(15, 23, 42, 28),
        field: rgba(255, 255, 255, 190),
        field_hover: rgba(255, 255, 255, 255),
        field_border: rgba(15, 23, 42, 20),
        text: rgba(0x1d, 0x23, 0x30, 255),
        text_muted: rgba(0x6b, 0x72, 0x80, 255),
        // BYU–Idaho blue.
        accent: rgba(0x00, 0x6e, 0xb6, 255),
        on_accent: Color::WHITE,
        active_tab: Color::WHITE,
        panel: rgba(0xf4, 0xf5, 0xf8, 255),
        panel_border: rgba(15, 23, 42, 34),
        selection: rgba(0x00, 0x6e, 0xb6, 64),
    }
}

fn dark() -> Palette {
    Palette {
        frame_top: rgba(0x24, 0x25, 0x2a, 255),
        frame_bottom: rgba(0x1b, 0x1c, 0x20, 255),
        shadow: rgba(0, 0, 0, 40),
        card_border: rgba(255, 255, 255, 20),
        icon: rgba(0xec, 0xef, 0xf5, 255),
        icon_disabled: rgba(0x7b, 0x82, 0x90, 255),
        hover: rgba(255, 255, 255, 18),
        pressed: rgba(255, 255, 255, 32),
        field: rgba(0x30, 0x33, 0x3c, 255),
        field_hover: rgba(0x39, 0x3d, 0x48, 255),
        field_border: rgba(0x67, 0x70, 0x83, 255),
        text: rgba(0xec, 0xee, 0xf2, 255),
        text_muted: rgba(0xb0, 0xb8, 0xc7, 255),
        accent: rgba(0x4d, 0xa3, 0xff, 255),
        on_accent: rgba(0x12, 0x1b, 0x29, 255),
        active_tab: rgba(0x41, 0x49, 0x59, 255),
        panel: rgba(0x29, 0x2c, 0x35, 255),
        panel_border: rgba(0x6e, 0x77, 0x89, 255),
        selection: rgba(0x4d, 0xa3, 0xff, 96),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(pixmap: &Pixmap, x: u32, y: u32) -> [u8; 4] {
        let p = pixmap.pixel(x, y).expect("pixel in bounds");
        [p.red(), p.green(), p.blue(), p.alpha()]
    }

    #[test]
    fn dark_palette_keeps_text_controls_and_focus_legible() {
        fn luminance(color: Color) -> f32 {
            let linear = |value: f32| {
                if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(color.red())
                + 0.7152 * linear(color.green())
                + 0.0722 * linear(color.blue())
        }
        let contrast = |a, b| {
            let (a, b) = (luminance(a), luminance(b));
            (a.max(b) + 0.05) / (a.min(b) + 0.05)
        };
        let dark = palette(true);
        for (foreground, background) in [
            (dark.text, dark.field_hover),
            (dark.text, dark.active_tab),
            (dark.text_muted, dark.panel),
            (dark.on_accent, dark.accent),
        ] {
            assert!(contrast(foreground, background) >= 4.5);
        }
        for (foreground, background) in [
            (dark.icon, dark.field_hover),
            (dark.icon_disabled, dark.frame_top),
            (dark.accent, dark.active_tab),
            (dark.accent, dark.field_hover),
            (dark.panel_border, dark.panel),
        ] {
            assert!(contrast(foreground, background) >= 3.0);
        }
        let light = palette(false);
        assert!(contrast(light.on_accent, light.accent) >= 4.5);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn loading_label_changes_only_address_field_and_matches_partial_paint() {
        let font = load_ui_font([
            "/System/Library/Fonts/SFNS.ttf",
            "/System/Library/Fonts/Helvetica.ttc",
        ])
        .expect("macOS UI font");
        for width in [420, 640, 1120] {
            for scale in [1.0, 1.5, 2.0] {
                for dark in [false, true] {
                    let width_px = (width as f64 * scale) as u32;
                    let height = (300.0 * scale) as u32;
                    let idle = ChromeState {
                        dark,
                        address_focused: true,
                        address_caret: usize::MAX,
                        ..ChromeState::default()
                    };
                    let busy = ChromeState {
                        loading: true,
                        ..idle
                    };
                    let address =
                        "https://example.com/a/very/long/address/that/must/not/cover/loading";
                    let before =
                        compose(width_px, height, scale, &idle, address, None, Some(&font))
                            .unwrap();
                    let after = compose(width_px, height, scale, &busy, address, None, Some(&font))
                        .unwrap();
                    assert_ne!(before.data(), after.data());
                    let strip =
                        compose_controls(width_px, height, scale, &busy, address, Some(&font))
                            .unwrap();
                    assert_eq!(strip.data(), &after.data()[..strip.data().len()]);
                    let rect = control_rect(Control::Address, width as f32).unwrap();
                    for y in 0..height {
                        for x in 0..width_px {
                            if (x as f64 / scale) < rect.left() as f64 - 1.0
                                || (x as f64 / scale) > rect.right() as f64 + 1.0
                                || (y as f64 / scale) < rect.top() as f64 - 1.0
                                || (y as f64 / scale) > rect.bottom() as f64 + 1.0
                            {
                                let i = ((y * width_px + x) * 4) as usize;
                                assert_eq!(&before.data()[i..i + 4], &after.data()[i..i + 4]);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn loading_bar_eases_along_the_bottom_of_the_address_field() {
        let start = loading_bar_fraction(std::time::Duration::ZERO);
        let mid = loading_bar_fraction(std::time::Duration::from_millis(400));
        let late = loading_bar_fraction(std::time::Duration::from_secs(4));
        assert!((0.08..0.2).contains(&start), "{start}");
        assert!(mid > start + 0.2, "{start} {mid}");
        assert!(late > mid);
        assert!(late < 0.94, "{late}");

        let width = 640.0;
        let field = control_rect(Control::Address, width).unwrap();
        let y =
            (field.bottom() - LOADING_BAR_INSET_BOTTOM - LOADING_BAR_HEIGHT / 2.0).round() as u32;
        for dark in [false, true] {
            let idle_state = ChromeState {
                dark,
                ..ChromeState::default()
            };
            let idle = compose(
                width as u32,
                200,
                1.0,
                &idle_state,
                "https://example.com/",
                None,
                None,
            )
            .unwrap();
            let early_state = ChromeState {
                loading: true,
                load_progress: start,
                ..idle_state
            };
            let later_state = ChromeState {
                load_progress: mid,
                ..early_state
            };
            let early = compose(
                width as u32,
                200,
                1.0,
                &early_state,
                "https://example.com/",
                None,
                None,
            )
            .unwrap();
            let later = compose(
                width as u32,
                200,
                1.0,
                &later_state,
                "https://example.com/",
                None,
                None,
            )
            .unwrap();
            let track = field.width() - LOADING_BAR_INSET_X * 2.0;
            let fill_start = field.left() + LOADING_BAR_INSET_X;
            let early_fill = (track * start).max(4.0);
            let body = (fill_start + early_fill * 0.2).round() as u32;
            let tip = (fill_start + early_fill * 0.92).round() as u32;
            assert!(
                bar_pixel_differs(pixel(&idle, body, y), pixel(&early, body, y)),
                "early bar {:?} vs {:?}",
                pixel(&early, body, y),
                pixel(&idle, body, y)
            );
            // The leading edge fades toward the empty field.
            let body_gap = color_distance(pixel(&early, body, y), pixel(&idle, body, y));
            let tip_gap = color_distance(pixel(&early, tip, y), pixel(&idle, tip, y));
            assert!(
                tip_gap + 12 < body_gap,
                "fade tip {tip_gap} should be closer to the field than the body {body_gap}"
            );
            let between = fill_start + track * (start + mid) / 2.0;
            let between = between.round() as u32;
            assert!(
                bar_pixel_differs(pixel(&early, between, y), pixel(&later, between, y)),
                "later bar {:?} vs early {:?}",
                pixel(&later, between, y),
                pixel(&early, between, y)
            );
            // The indicator stays inside the address field.
            assert_eq!(
                pixel(&idle, 20, CENTER_Y as u32),
                pixel(&early, 20, CENTER_Y as u32)
            );
            let below = (field.bottom() + 6.0) as u32;
            assert_eq!(
                pixel(&idle, body, below),
                pixel(&early, body, below),
                "bar spilled below the address field"
            );
        }
    }

    fn bar_pixel_differs(idle: [u8; 4], bar: [u8; 4]) -> bool {
        let sum = |pixel: [u8; 4]| u32::from(pixel[0]) + u32::from(pixel[1]) + u32::from(pixel[2]);
        bar[2] > idle[2].saturating_add(40) || sum(bar) + 120 < sum(idle)
    }

    fn color_distance(left: [u8; 4], right: [u8; 4]) -> u32 {
        left.into_iter()
            .zip(right)
            .map(|(a, b)| u32::from(a.abs_diff(b)))
            .sum()
    }

    #[test]
    fn page_card_is_inset_below_toolbar() {
        assert_eq!(
            page_rect(640, 480, 1.0),
            Some(PixelRect {
                x: 8,
                y: 88,
                width: 624,
                height: 384
            })
        );
        assert_eq!(
            page_rect(1280, 960, 2.0).map(|r| (r.x, r.y)),
            Some((16, 176))
        );
        assert_eq!(page_rect(640, 60, 1.0), None);
        assert_eq!(page_rect(16, 480, 1.0), None);
    }

    #[test]
    fn page_is_clipped_to_a_rounded_card() {
        for scale in [1.0, 2.0] {
            let (width, height) = ((640.0 * scale) as u32, (480.0 * scale) as u32);
            let card = page_rect(width, height, scale).unwrap();
            let mut page = Pixmap::new(card.width, card.height).unwrap();
            page.fill(Color::from_rgba8(255, 0, 0, 255));
            let state = ChromeState::default();
            let out = compose(
                width,
                height,
                scale,
                &state,
                "byui://welcome",
                Some(&page),
                None,
            )
            .unwrap();
            let middle = pixel(&out, card.x + card.width / 2, card.y + card.height / 2);
            assert_eq!(middle, [255, 0, 0, 255]);
            // The rounded corner leaves the frame showing at the card's corner.
            assert_ne!(pixel(&out, card.x, card.y), [255, 0, 0, 255]);
            assert_eq!(pixel(&out, 0, height - 1)[3], 255);
        }
    }

    #[test]
    fn hit_testing_finds_toolbar_controls() {
        // A 600-pixel window keeps the address field at its leftmost spot,
        // so the navigation buttons sit at their leading position.
        assert_eq!(hit_test(26.0, CENTER_Y as f64, 600.0), Some(Control::Back));
        assert_eq!(
            hit_test(90.0, CENTER_Y as f64, 600.0),
            Some(Control::Reload)
        );
        assert_eq!(
            hit_test(500.0, CENTER_Y as f64, 1000.0),
            Some(Control::Address)
        );
        assert_eq!(
            hit_test(1000.0 - 30.0, CENTER_Y as f64, 1000.0),
            Some(Control::Menu)
        );
        assert_eq!(hit_test(109.0, CENTER_Y as f64, 600.0), None);
        // Wide windows center the address field but keep navigation left.
        assert_eq!(hit_test(26.0, CENTER_Y as f64, 1000.0), Some(Control::Back));
        assert_eq!(hit_test(500.0, 200.0, 1000.0), None);
        let state = ChromeState::default();
        assert!(!Control::Back.is_enabled(&state));
        assert!(Control::Reload.is_enabled(&state));
    }

    #[test]
    fn site_info_has_a_separate_hit_region_before_address_text() {
        for width in [420.0, 1120.0] {
            let field = control_rect(Control::Address, width as f32).unwrap();
            let button = control_rect(Control::SiteInfo, width as f32).unwrap();
            let text_x = field.left() + ADDRESS_TEXT_LEADING;
            assert!(button.right() < text_x);
            assert_eq!(
                hit_test(f64::from(button.left() + 1.0), f64::from(CENTER_Y), width),
                Some(Control::SiteInfo)
            );
            assert_eq!(
                hit_test(f64::from(text_x), f64::from(CENTER_Y), width),
                Some(Control::Address)
            );
        }
    }

    #[test]
    fn disabled_navigation_controls_still_occupy_hit_regions() {
        let back = hit_test(26.0, CENTER_Y as f64, 600.0);
        let forward = hit_test(58.0, CENTER_Y as f64, 600.0);

        assert_eq!(back, Some(Control::Back));
        assert_eq!(forward, Some(Control::Forward));
        let state = ChromeState::default();
        assert!(!back.is_some_and(|control| control.is_enabled(&state)));
        assert!(!forward.is_some_and(|control| control.is_enabled(&state)));
    }

    #[test]
    fn navigation_buttons_stay_left_under_the_traffic_lights() {
        for width in [200.0, 600.0, 1120.0, 2000.0] {
            let back = control_rect(Control::Back, width).unwrap();
            let forward = control_rect(Control::Forward, width).unwrap();
            let reload = control_rect(Control::Reload, width).unwrap();
            assert_eq!(back.x(), NAV_LEADING);
            assert_eq!(forward.x() - back.x(), NAV_STRIDE);
            assert_eq!(reload.x() - forward.x(), NAV_STRIDE);
            if let Some(field) = control_rect(Control::Address, width) {
                assert!(field.x() - reload.right() >= NAV_ADDRESS_GAP);
            }
        }
        assert_eq!(
            hit_test(f64::from(NAV_LEADING) + 1.0, CENTER_Y as f64, 1000.0),
            Some(Control::Back)
        );
        assert_eq!(hit_test(2.0, CENTER_Y as f64, 1000.0), None);
        // The address field never slides under the navigation buttons, and
        // still centers in wide windows.
        let narrow = control_rect(Control::Address, 400.0).unwrap();
        assert_eq!(narrow.x(), ADDRESS_LEADING);
        let centered = control_rect(Control::Address, 2000.0).unwrap();
        assert_eq!(centered.x(), (2000.0 - ADDRESS_MAX_WIDTH) / 2.0);
        assert!(control_rect(Control::Address, 200.0).is_none());
    }

    #[test]
    fn pointer_positions_map_to_address_character_boundaries() {
        let font = load_ui_font([
            "/System/Library/Fonts/SFNS.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "C:/Windows/Fonts/segoeui.ttf",
        ]);
        let Some(font) = font else {
            eprintln!("skipping: system fonts unavailable");
            return;
        };
        let address = "byui://welcome";
        let field = control_rect(Control::Address, 1000.0).unwrap();
        let text_x = f64::from(field.x() + ADDRESS_TEXT_LEADING);
        let at = |x: f64| address_offset_at(x, 1000.0, 2.0, Some(&font), address);
        assert_eq!(at(f64::from(field.x())), 0);
        assert_eq!(at(f64::from(field.right())), address.len());
        // The pen position after "byui" maps back to offset 4.
        let after = f64::from(text_advance(&font, TEXT_SIZE * 2.0, "byui") / 2.0);
        assert_eq!(at(text_x + after), 4);
        // Positions increase monotonically across the text.
        let offsets: Vec<_> = (0..200).map(|step| at(text_x + f64::from(step))).collect();
        assert!(offsets.windows(2).all(|pair| pair[0] <= pair[1]));
        // Without a font every point maps to the start.
        assert_eq!(
            address_offset_at(text_x + 50.0, 1000.0, 1.0, None, address),
            0
        );
    }

    #[test]
    fn toolbar_plus_is_removed_and_controls_stay_below_tabs() {
        for width in [420.0, 640.0, 1120.0] {
            assert_eq!(hit_test(width - 66.0, CENTER_Y as f64, width), None);
            for control in CONTROLS {
                assert!(
                    control_rect(control, width as f32).unwrap().top()
                        >= crate::tab_strip::HEIGHT as f32
                );
            }
        }
        assert_eq!(
            CONTROLS,
            [
                Control::Back,
                Control::Forward,
                Control::Reload,
                Control::Menu,
                Control::SiteInfo,
                Control::Address,
            ]
        );
    }

    #[test]
    fn narrow_windows_drop_trailing_controls_then_the_address_field() {
        assert!(control_rect(Control::Menu, 340.0).is_none());
        assert!(control_rect(Control::Address, 340.0).is_some());
        assert!(control_rect(Control::Address, 200.0).is_none());
        let wide = control_rect(Control::Address, 2000.0).unwrap();
        assert_eq!(wide.width(), ADDRESS_MAX_WIDTH);
        assert_eq!(wide.x(), (2000.0 - ADDRESS_MAX_WIDTH) / 2.0);
    }

    #[test]
    fn every_state_and_size_composes_safely() {
        assert!(
            compose(
                0,
                0,
                1.0,
                &ChromeState::default(),
                "byui://welcome",
                None,
                None
            )
            .is_none()
        );
        for (width, height) in [(1, 1), (20, 60), (300, 120), (1200, 800)] {
            for dark in [false, true] {
                for hovered in [None, Some(Control::Reload), Some(Control::Address)] {
                    let state = ChromeState {
                        hovered,
                        pressed: hovered,
                        dark,
                        loading: false,
                        load_progress: 0.0,
                        can_go_back: false,
                        can_go_forward: false,
                        find_open: false,
                        address_focused: false,
                        address_anchor: None,
                        address_caret: 0,
                    };
                    let out =
                        compose(width, height, 1.5, &state, "byui://welcome", None, None).unwrap();
                    assert_eq!(out.width(), width);
                }
            }
        }
    }

    #[test]
    fn controls_strip_matches_full_composition() {
        for (scale, dark) in [(1.0, false), (2.0, true), (1.5, false)] {
            let (width, height) = ((640.0 * scale) as u32, (400.0 * scale) as u32);
            let state = ChromeState {
                hovered: Some(Control::Reload),
                dark,
                ..ChromeState::default()
            };
            let full = compose(width, height, scale, &state, "byui://welcome", None, None).unwrap();
            let strip =
                compose_controls(width, height, scale, &state, "byui://welcome", None).unwrap();
            assert_eq!(strip.height(), controls_height_in_pixels(scale));
            assert_eq!(strip.data(), &full.data()[..strip.data().len()]);
            // The strip ends above the card shadow.
            let shadow_top = page_rect(width, height, scale).unwrap().y - (4.0 * scale) as u32;
            assert!(strip.height() <= shadow_top);
        }
    }

    #[test]
    fn dark_and_light_frames_differ() {
        let light = compose(
            400,
            200,
            1.0,
            &ChromeState::default(),
            "byui://welcome",
            None,
            None,
        )
        .unwrap();
        let dark_state = ChromeState {
            dark: true,
            ..ChromeState::default()
        };
        let dark = compose(400, 200, 1.0, &dark_state, "byui://welcome", None, None).unwrap();
        assert!(pixel(&light, 200, 4)[0] > 200);
        assert!(pixel(&dark, 200, 4)[0] < 60);
    }

    #[test]
    fn focused_address_field_has_distinct_chrome() {
        let normal = compose(
            400,
            200,
            1.0,
            &ChromeState::default(),
            "byui://welcome",
            None,
            None,
        )
        .unwrap();
        let focused = compose(
            400,
            200,
            1.0,
            &ChromeState {
                address_focused: true,
                ..ChromeState::default()
            },
            "byui://welcome",
            None,
            None,
        )
        .unwrap();
        assert_ne!(normal.data(), focused.data());
    }

    #[test]
    fn focused_address_shows_a_caret_in_both_palettes() {
        for dark in [false, true] {
            let focused = ChromeState {
                dark,
                address_focused: true,
                ..ChromeState::default()
            };
            let selected = ChromeState {
                address_anchor: Some(0),
                address_caret: 1,
                ..focused
            };
            let with_caret = compose(400, 200, 1.0, &focused, "x", None, None).unwrap();
            let without = compose(400, 200, 1.0, &selected, "x", None, None).unwrap();
            let rect = control_rect(Control::Address, 400.0).unwrap();
            // With no font the caret sits at the text origin.
            let x = (rect.x() + 31.5) as u32;
            let caret = pixel(&with_caret, x, CENTER_Y as u32);
            let field = pixel(&without, x, CENTER_Y as u32);
            // The caret uses the text color, so it contrasts with the field.
            let lightness = |p: [u8; 4]| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]);
            if dark {
                assert!(
                    lightness(caret) > lightness(field) + 300,
                    "{caret:?} {field:?}"
                );
            } else {
                assert!(
                    lightness(caret) + 300 < lightness(field),
                    "{caret:?} {field:?}"
                );
            }
        }
    }
}

/// Hit-tests completion rows in logical window coordinates.
pub(crate) fn suggestion_at(width: f32, cursor: Option<(f64, f64)>, count: usize) -> Option<usize> {
    let field = control_rect(Control::Address, width)?;
    let (x, y) = cursor?;
    let y = y as f32 - field.bottom() - 4.0;
    if x < field.left() as f64 || x >= field.right() as f64 || y < 0.0 {
        return None;
    }
    let index = (y / 38.0) as usize;
    (index < count).then_some(index)
}

/// Paints local completions over the page after all other chrome layers.
pub(crate) fn draw_suggestions(
    pixmap: &mut Pixmap,
    scale: f32,
    state: &ChromeState,
    suggestions: &[(String, String, Option<chrome::TabId>)],
    selected: Option<usize>,
    font: Option<&FontVec>,
) {
    if !state.address_focused || suggestions.is_empty() {
        return;
    }
    let Some(field) = control_rect(Control::Address, pixmap.width() as f32 / scale) else {
        return;
    };
    let transform = Transform::from_scale(scale, scale);
    let top = field.bottom() + 4.0;
    let background = if state.dark {
        Color::from_rgba8(43, 44, 48, 255)
    } else {
        Color::from_rgba8(250, 250, 252, 255)
    };
    if let Some(path) = rounded_rect(
        field.left(),
        top,
        field.width(),
        suggestions.len() as f32 * 38.0,
        8.0,
    ) {
        fill(pixmap, &path, background, transform);
    }
    for (index, (label, _, _)) in suggestions.iter().enumerate() {
        let y = top + index as f32 * 38.0;
        if selected == Some(index)
            && let Some(path) =
                rounded_rect(field.left() + 3.0, y + 2.0, field.width() - 6.0, 34.0, 5.0)
        {
            fill(
                pixmap,
                &path,
                if state.dark {
                    Color::from_rgba8(67, 72, 88, 255)
                } else {
                    Color::from_rgba8(219, 229, 250, 255)
                },
                transform,
            );
        }
        if let Some(font) = font {
            draw_text(
                pixmap,
                font,
                13.0 * scale,
                (field.left() + 14.0) * scale,
                (y + 24.0) * scale,
                (field.right() - 12.0) * scale,
                if state.dark {
                    Color::from_rgba8(235, 236, 240, 255)
                } else {
                    Color::from_rgba8(32, 34, 40, 255)
                },
                label,
            );
        }
    }
}

#[cfg(test)]
mod completion_tests {
    use super::*;

    #[test]
    fn completions_paint_and_hit_test_the_same_rows_at_each_scale() {
        for scale in [1.0, 2.0] {
            for dark in [false, true] {
                let state = ChromeState {
                    address_focused: true,
                    dark,
                    ..ChromeState::default()
                };
                let rows = vec![("Example".into(), "https://example.com".into(), None)];
                let mut image = compose(
                    (1000.0 * scale) as u32,
                    (600.0 * scale) as u32,
                    scale as f64,
                    &state,
                    "exam",
                    None,
                    None,
                )
                .unwrap();
                let before = image.clone();
                draw_suggestions(&mut image, scale, &state, &rows, Some(0), None);
                let field = control_rect(Control::Address, 1000.0).unwrap();
                let x = field.left() + 20.0;
                let y = field.bottom() + 20.0;
                assert_eq!(
                    suggestion_at(1000.0, Some((x as f64, y as f64)), 1),
                    Some(0)
                );
                assert_ne!(
                    image.pixel((x * scale) as u32, (y * scale) as u32),
                    before.pixel((x * scale) as u32, (y * scale) as u32)
                );
                assert_eq!(
                    suggestion_at(1000.0, Some((x as f64, (y + 38.0) as f64)), 1),
                    None
                );
            }
        }
    }
}
