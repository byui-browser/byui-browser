//! Browser chrome for the native shell: one unified toolbar row (navigation,
//! address field, actions) above a rounded page card.
//!
//! Window controls come from the operating system: each shell keeps its native
//! decorations, and where those overlap the toolbar (macOS's traffic lights on
//! a transparent titlebar) the controls shift right by
//! [`ChromeState::window_controls_inset`]. Navigation and tabs remain staged;
//! the address field supports focus and editing, while only the built-in
//! `byui://welcome` page can currently be submitted.

use ab_glyph::{Font, FontVec, PxScale, ScaleFont, point};
use tiny_skia::{Color, FillRule, Paint, Path, PathBuilder, Pixmap, Rect, Stroke, Transform};

/// Toolbar height in logical pixels; the page card starts directly below it.
pub(crate) const TOOLBAR_HEIGHT: f64 = 52.0;
/// Logical height of the strip holding every toolbar control. It stops above
/// the page card's shadow so the strip can be repainted on its own.
const CONTROLS_HEIGHT: f64 = 46.0;
/// Gap in logical pixels between the page card and the window's side and
/// bottom edges.
const PAGE_INSET: f64 = 8.0;
const PAGE_RADIUS: f32 = 10.0;
const CENTER_Y: f32 = 26.0;
const BUTTON_SIZE: f32 = 28.0;
const BUTTON_TOP: f32 = CENTER_Y - BUTTON_SIZE / 2.0;
const ADDRESS_HEIGHT: f32 = 34.0;
const ADDRESS_MAX_WIDTH: f32 = 640.0;
/// Logical distance from the toolbar's left edge (after the window-controls
/// inset) to the Back button.
const NAV_LEADING: f32 = 12.0;
/// Logical distance from the toolbar's left edge (after the window-controls
/// inset) to the earliest position of the address field.
const ADDRESS_LEADING: f32 = 116.0;
/// Narrower windows drop the New Tab and menu buttons to keep the address
/// field usable.
const TRAILING_MIN_WIDTH: f32 = 360.0;
const TEXT_SIZE: f32 = 13.0;

/// A clickable region of the chrome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Control {
    /// Back navigation; disabled because there is no history yet.
    Back,
    /// Forward navigation; disabled because there is no history yet.
    Forward,
    /// Reload; fetches the current URL to the terminal and repaints the loaded page.
    Reload,
    /// Address field; supports staged text editing.
    Address,
    /// New tab placeholder; tabs are not implemented yet.
    NewTab,
    /// Overflow menu placeholder.
    Menu,
}

const CONTROLS: [Control; 6] = [
    Control::Back,
    Control::Forward,
    Control::Reload,
    Control::NewTab,
    Control::Menu,
    Control::Address,
];

impl Control {
    /// Whether the control responds to hover and clicks.
    pub(crate) fn is_enabled(self) -> bool {
        !matches!(self, Control::Back | Control::Forward)
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
    /// Logical width at the toolbar's left reserved for the operating
    /// system's own window buttons when they overlap the toolbar, as macOS's
    /// traffic lights do on a transparent titlebar. Zero when the system draws
    /// its buttons in a separate titlebar above the toolbar.
    pub window_controls_inset: f32,
    /// Whether the address field owns keyboard input.
    pub address_focused: bool,
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
    let top = (TOOLBAR_HEIGHT * scale_factor).round() as u32;
    let page_width = width.checked_sub(inset * 2).filter(|w| *w > 0)?;
    let page_height = height.checked_sub(top + inset).filter(|h| *h > 0)?;
    Some(PixelRect {
        x: inset,
        y: top,
        width: page_width,
        height: page_height,
    })
}

/// Returns the enabled or disabled control at a point given in logical
/// pixels, for a window `width` logical pixels wide whose toolbar starts
/// `inset` logical pixels in (see [`ChromeState::window_controls_inset`]).
pub(crate) fn hit_test(x: f64, y: f64, width: f64, inset: f32) -> Option<Control> {
    let (x, y) = (x as f32, y as f32);
    CONTROLS.into_iter().find(|control| {
        control_rect(*control, width as f32, inset).is_some_and(|rect| {
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

    candidates.into_iter().find_map(|path| {
        let mut font = FontVec::try_from_vec(std::fs::read(path).ok()?).ok()?;
        // SF and other variable UI fonts pick the optical size designed for
        // small text; fonts without an `opsz` axis ignore this.
        font.set_variation(b"opsz", TEXT_SIZE);
        Some(font)
    })
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
    if let Some(card) = page_rect(width, height, scale_factor) {
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
        for pixel in row.chunks_exact_mut(4) {
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
    let inset = state.window_controls_inset;
    for control in CONTROLS {
        if control == Control::Address {
            continue;
        }
        let Some(rect) = control_rect(control, width, inset) else {
            continue;
        };
        let active = control.is_enabled();
        let background = match (state.pressed, state.hovered) {
            (Some(pressed), _) if pressed == control && active => Some(palette.pressed),
            (_, Some(hovered)) if hovered == control && active => Some(palette.hover),
            _ => None,
        };
        if let Some(color) = background {
            if let Some(path) = rounded_rect(rect.x(), rect.y(), rect.width(), rect.height(), 7.0) {
                fill(pixmap, &path, color, transform);
            }
        }
        let color = if active {
            palette.icon
        } else {
            palette.icon_disabled
        };
        let (cx, cy) = (rect.x() + rect.width() / 2.0, CENTER_Y);
        draw_icon(pixmap, control, cx, cy, color, transform);
    }
    if let Some(rect) = control_rect(Control::Address, width, inset) {
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
        Control::NewTab => {
            builder.move_to(cx - 6.0, cy);
            builder.line_to(cx + 6.0, cy);
            builder.move_to(cx, cy - 6.0);
            builder.line_to(cx, cy + 6.0);
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
    let lock_x = rect.x() + 16.0;
    if let Some(body) = rounded_rect(lock_x - 4.0, cy - 1.5, 8.0, 6.5, 1.6) {
        fill(pixmap, &body, palette.accent, transform);
    }
    let mut shackle = PathBuilder::new();
    let (r, top) = (2.5, cy - 3.6);
    shackle.move_to(lock_x - r, cy - 1.0);
    shackle.line_to(lock_x - r, top);
    shackle.cubic_to(
        lock_x - r,
        top - r * 4.0 / 3.0,
        lock_x + r,
        top - r * 4.0 / 3.0,
        lock_x + r,
        top,
    );
    shackle.line_to(lock_x + r, cy - 1.0);
    if let Some(path) = shackle.finish() {
        stroke(pixmap, &path, palette.accent, 1.4, transform);
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

    if let Some(font) = font {
        let size = TEXT_SIZE * scale;
        let scaled = font.as_scaled(PxScale::from(size));
        let baseline = cy * scale + (scaled.ascent() + scaled.descent()) / 2.0;
        let clip = (star_x - 12.0) * scale;
        draw_text(
            pixmap,
            font,
            size,
            (rect.x() + 30.0) * scale,
            baseline,
            clip,
            if state.address_focused {
                palette.text
            } else {
                palette.text_muted
            },
            address,
        );
    }
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

/// Returns where `control` sits in a toolbar `width` logical pixels wide
/// whose leading controls start `inset` pixels in, or `None` when the window
/// is too narrow to show it.
fn control_rect(control: Control, width: f32, inset: f32) -> Option<Rect> {
    let button = |x: f32| Rect::from_xywh(x, BUTTON_TOP, BUTTON_SIZE, BUTTON_SIZE);
    let trailing = width >= TRAILING_MIN_WIDTH;
    match control {
        Control::Back => button(inset + NAV_LEADING),
        Control::Forward => button(inset + NAV_LEADING + 32.0),
        Control::Reload => button(inset + NAV_LEADING + 64.0),
        Control::NewTab => button(width - 80.0).filter(|_| trailing),
        Control::Menu => button(width - 44.0).filter(|_| trailing),
        Control::Address => {
            let leading = inset + ADDRESS_LEADING;
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
    }
}

fn rounded_rect(x: f32, y: f32, w: f32, h: f32, radius: f32) -> Option<Path> {
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

fn fill(pixmap: &mut Pixmap, path: &Path, color: Color, transform: Transform) {
    let mut paint = Paint::default();
    paint.set_color(color);
    pixmap.fill_path(path, &paint, FillRule::Winding, transform, None);
}

fn stroke(pixmap: &mut Pixmap, path: &Path, color: Color, width: f32, transform: Transform) {
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

fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color::from_rgba8(r, g, b, a)
}

struct Palette {
    frame_top: Color,
    frame_bottom: Color,
    shadow: Color,
    card_border: Color,
    icon: Color,
    icon_disabled: Color,
    hover: Color,
    pressed: Color,
    field: Color,
    field_hover: Color,
    field_border: Color,
    text: Color,
    text_muted: Color,
    accent: Color,
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
    }
}

fn dark() -> Palette {
    Palette {
        frame_top: rgba(0x24, 0x25, 0x2a, 255),
        frame_bottom: rgba(0x1b, 0x1c, 0x20, 255),
        shadow: rgba(0, 0, 0, 40),
        card_border: rgba(255, 255, 255, 20),
        icon: rgba(0xe2, 0xe5, 0xeb, 255),
        icon_disabled: rgba(0xe2, 0xe5, 0xeb, 70),
        hover: rgba(255, 255, 255, 18),
        pressed: rgba(255, 255, 255, 32),
        field: rgba(255, 255, 255, 16),
        field_hover: rgba(255, 255, 255, 26),
        field_border: rgba(255, 255, 255, 16),
        text: rgba(0xec, 0xee, 0xf2, 255),
        text_muted: rgba(0x96, 0x9c, 0xa8, 255),
        accent: rgba(0x4d, 0xa3, 0xff, 255),
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
    fn page_card_is_inset_below_toolbar() {
        assert_eq!(
            page_rect(640, 480, 1.0),
            Some(PixelRect {
                x: 8,
                y: 52,
                width: 624,
                height: 420
            })
        );
        assert_eq!(
            page_rect(1280, 960, 2.0).map(|r| (r.x, r.y)),
            Some((16, 104))
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
        assert_eq!(hit_test(26.0, 26.0, 1000.0, 0.0), Some(Control::Back));
        assert_eq!(hit_test(90.0, 26.0, 1000.0, 0.0), Some(Control::Reload));
        assert_eq!(hit_test(500.0, 26.0, 1000.0, 0.0), Some(Control::Address));
        assert_eq!(
            hit_test(1000.0 - 30.0, 26.0, 1000.0, 0.0),
            Some(Control::Menu)
        );
        assert_eq!(hit_test(109.0, 26.0, 1000.0, 0.0), None);
        assert_eq!(hit_test(500.0, 200.0, 1000.0, 0.0), None);
        assert!(!Control::Back.is_enabled());
        assert!(Control::Reload.is_enabled());
    }

    #[test]
    fn disabled_navigation_controls_still_occupy_hit_regions() {
        let back = hit_test(26.0, 26.0, 1000.0, 0.0);
        let forward = hit_test(58.0, 26.0, 1000.0, 0.0);

        assert_eq!(back, Some(Control::Back));
        assert_eq!(forward, Some(Control::Forward));
        assert!(!back.is_some_and(Control::is_enabled));
        assert!(!forward.is_some_and(Control::is_enabled));
    }

    #[test]
    fn window_controls_inset_shifts_leading_controls() {
        // Space reserved for macOS's traffic lights holds no toolbar control.
        assert_eq!(hit_test(20.0, 26.0, 1000.0, 72.0), None);
        assert_eq!(hit_test(60.0, 26.0, 1000.0, 72.0), None);
        assert_eq!(hit_test(98.0, 26.0, 1000.0, 72.0), Some(Control::Back));
        assert_eq!(hit_test(162.0, 26.0, 1000.0, 72.0), Some(Control::Reload));
        assert_eq!(hit_test(181.0, 26.0, 1000.0, 72.0), None);
        // The address field never slides under the leading controls.
        let narrow = control_rect(Control::Address, 400.0, 72.0).unwrap();
        assert_eq!(narrow.x(), 72.0 + ADDRESS_LEADING);
        let centered = control_rect(Control::Address, 2000.0, 72.0).unwrap();
        assert_eq!(centered.x(), (2000.0 - ADDRESS_MAX_WIDTH) / 2.0);
    }

    #[test]
    fn narrow_windows_drop_trailing_controls_then_the_address_field() {
        assert!(control_rect(Control::Menu, 340.0, 0.0).is_none());
        assert!(control_rect(Control::Address, 340.0, 0.0).is_some());
        assert!(control_rect(Control::Address, 200.0, 0.0).is_none());
        let wide = control_rect(Control::Address, 2000.0, 0.0).unwrap();
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
                        address_focused: false,
                        window_controls_inset: if dark { 72.0 } else { 0.0 },
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
}
