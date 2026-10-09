//! Toolbar with disabled Back/Forward/Reload buttons; navigation is not implemented.

const HEIGHT: f64 = 48.0; // Logical pixels, including the bottom divider.
const BACKGROUND: u32 = 0x00efefef;
const DIVIDER: u32 = 0x00cccccc;
const CONTENT: u32 = 0x00ffffff;
const BUTTON_BACKGROUND: u32 = 0x00e7e7e7;
const BUTTON_BORDER: u32 = 0x00c8c8c8;
const DISABLED_ARROW: u32 = 0x00999999;
const BUTTON_SIZE: f64 = 32.0;
const BUTTON_TOP: f64 = 8.0;
const BACK_LEFT: f64 = 8.0;
const FORWARD_LEFT: f64 = 48.0;
const RELOAD_LEFT: f64 = 88.0;

/// Returns the toolbar's height in device pixels for a window's display scale.
pub(super) fn height_in_pixels(scale_factor: f64) -> usize {
    (HEIGHT * scale_factor).round() as usize
}

/// Paints a full-width toolbar and blank content into an RGB window buffer.
/// `width` is in device pixels; `scale_factor` converts logical to device pixels.
/// All buttons are disabled placeholders with no click or keyboard action.
pub(super) fn draw(pixels: &mut [u32], width: usize, scale_factor: f64) {
    pixels.fill(CONTENT);
    if width == 0 {
        return;
    }
    let toolbar_height = height_in_pixels(scale_factor);
    let divider_height = (scale_factor.round() as usize).max(1);
    for (y, row) in pixels.chunks_mut(width).take(toolbar_height).enumerate() {
        row.fill(if y >= toolbar_height.saturating_sub(divider_height) {
            DIVIDER
        } else {
            BACKGROUND
        });
        for (x, pixel) in row.iter_mut().enumerate() {
            let local_y = (y as f64 + 0.5 - BUTTON_TOP * scale_factor) / scale_factor;
            for (left, forward) in [(BACK_LEFT, false), (FORWARD_LEFT, true)] {
                let local_x = (x as f64 + 0.5 - left * scale_factor) / scale_factor;
                if let Some(color) = button_pixel(local_x, local_y, forward) {
                    *pixel = color;
                }
            }
            let reload_x = (x as f64 + 0.5 - RELOAD_LEFT * scale_factor) / scale_factor;
            if let Some(color) = reload_pixel(reload_x, local_y) {
                *pixel = color;
            }
        }
    }
}

// Coordinates are local logical pixels. Mirror the same arrow for Forward.
fn button_pixel(x: f64, y: f64, forward: bool) -> Option<u32> {
    if !(0.0..BUTTON_SIZE).contains(&x) || !(0.0..BUTTON_SIZE).contains(&y) {
        return None;
    }
    if x <= 1.0 || x >= BUTTON_SIZE - 1.0 || y <= 1.0 || y >= BUTTON_SIZE - 1.0 {
        return Some(BUTTON_BORDER);
    }
    let x = if forward { BUTTON_SIZE - x } else { x };
    let dy = (y - 16.0).abs();
    let shaft = (9.0..24.0).contains(&x) && dy <= 1.0;
    let head = (8.0..17.0).contains(&x) && (dy - (x - 8.0)).abs() <= 1.0;
    Some(if shaft || head {
        DISABLED_ARROW
    } else {
        BUTTON_BACKGROUND
    })
}

// UI-only Reload placeholder; page loading and reload actions are not implemented.
fn reload_pixel(x: f64, y: f64) -> Option<u32> {
    if !(0.0..BUTTON_SIZE).contains(&x) || !(0.0..BUTTON_SIZE).contains(&y) {
        return None;
    }
    if x <= 1.0 || x >= BUTTON_SIZE - 1.0 || y <= 1.0 || y >= BUTTON_SIZE - 1.0 {
        return Some(BUTTON_BORDER);
    }
    let dx = x - 16.0;
    let dy = y - 16.0;
    let radius_squared = dx * dx + dy * dy;
    // Leave a gap at the upper right and end the clockwise arc with an arrowhead.
    let arc = (49.0..=81.0).contains(&radius_squared) && !(dx > 0.0 && dy < -2.0);
    let head = (10.0..=17.0).contains(&y) && (x - 24.0).abs() <= (17.0 - y) * 0.65;
    Some(if arc || head {
        DISABLED_ARROW
    } else {
        BUTTON_BACKGROUND
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reload_follows_forward_and_scales_with_the_existing_buttons() {
        assert_eq!(RELOAD_LEFT - FORWARD_LEFT, FORWARD_LEFT - BACK_LEFT);
        for scale in [1.0, 1.5, 2.0] {
            let width = (140.0 * scale) as usize;
            let mut pixels = vec![0; width * (60.0 * scale) as usize];
            draw(&mut pixels, width, scale);
            let sample =
                |x: f64, y: f64| pixels[(y * scale) as usize * width + (x * scale) as usize];
            assert_eq!(sample(RELOAD_LEFT, BUTTON_TOP), BUTTON_BORDER);
            assert_eq!(
                sample(RELOAD_LEFT + 2.0, BUTTON_TOP + 2.0),
                BUTTON_BACKGROUND
            );
            assert_eq!(sample(RELOAD_LEFT + 8.0, BUTTON_TOP + 16.0), DISABLED_ARROW);
            assert_eq!(
                sample(RELOAD_LEFT + 24.0, BUTTON_TOP + 12.0),
                DISABLED_ARROW
            );
            assert_eq!(
                sample(RELOAD_LEFT + 16.0, BUTTON_TOP + 16.0),
                BUTTON_BACKGROUND
            );
            assert_eq!(
                sample(RELOAD_LEFT + 19.0, BUTTON_TOP + 9.0),
                BUTTON_BACKGROUND
            );
            assert_eq!(sample(RELOAD_LEFT - 4.0, BUTTON_TOP + 16.0), BACKGROUND);
            assert_eq!(
                sample(RELOAD_LEFT + BUTTON_SIZE, BUTTON_TOP + 16.0),
                BACKGROUND
            );
        }
    }

    #[test]
    fn clipping_reload_does_not_change_existing_buttons_or_content() {
        let mut full = vec![0; 140 * 60];
        draw(&mut full, 140, 1.0);
        for width in [88, 90, 105, 119] {
            let mut clipped = vec![0; width * 60];
            draw(&mut clipped, width, 1.0);
            for (y, row) in clipped.chunks(width).enumerate() {
                assert_eq!(row, &full[y * 140..y * 140 + width]);
            }
            assert!(clipped[width * 48..].iter().all(|p| *p == CONTENT));
        }
    }

    #[test]
    fn toolbar_spans_width_and_reserves_content_at_each_scale() {
        for scale in [1.0, 1.5, 2.0] {
            for width in [1, 17, 640] {
                let height = (HEIGHT * scale).round() as usize;
                let divider = (scale.round() as usize).max(1);
                let mut pixels = vec![0; width * (height + 10)];
                draw(&mut pixels, width, scale);
                // The top margin remains clear above the buttons.
                let margin = (BUTTON_TOP * scale) as usize;
                assert!(pixels[..width * margin].iter().all(|p| *p == BACKGROUND));
                assert!(
                    pixels[width * (height - divider)..width * height]
                        .iter()
                        .all(|p| *p == DIVIDER)
                );
                assert!(pixels[width * height..].iter().all(|p| *p == CONTENT));
            }
        }
    }

    #[test]
    fn back_and_forward_are_visible_mirrored_buttons_at_each_scale() {
        for scale in [1.0, 1.5, 2.0] {
            let width = (100.0 * scale) as usize;
            let mut pixels = vec![0; width * (60.0 * scale) as usize];
            draw(&mut pixels, width, scale);
            let size = (BUTTON_SIZE * scale) as usize;
            let top = (BUTTON_TOP * scale) as usize;
            let back = (BACK_LEFT * scale) as usize;
            let forward = (FORWARD_LEFT * scale) as usize;
            let mut arrow_pixels = 0;
            for y in 0..size {
                for x in 0..size {
                    let back_pixel = pixels[(top + y) * width + back + x];
                    let forward_pixel = pixels[(top + y) * width + forward + size - 1 - x];
                    assert_eq!(back_pixel, forward_pixel);
                    arrow_pixels += usize::from(back_pixel == DISABLED_ARROW);
                }
            }
            assert!(arrow_pixels > 0);
            // Back's tip is on its left; the shaft extends to its right.
            let sample =
                |x: f64, y: f64| pixels[(y * scale) as usize * width + (x * scale) as usize];
            assert_eq!(sample(BACK_LEFT + 8.0, BUTTON_TOP + 16.0), DISABLED_ARROW);
            assert_eq!(sample(BACK_LEFT + 23.0, BUTTON_TOP + 16.0), DISABLED_ARROW);
            assert_eq!(
                sample(BACK_LEFT + 23.0, BUTTON_TOP + 10.0),
                BUTTON_BACKGROUND
            );
        }
    }

    #[test]
    fn narrow_window_clips_buttons_without_wrapping_into_other_rows() {
        let mut pixels = vec![0; 20 * 60];
        draw(&mut pixels, 20, 1.0);
        for row in pixels.chunks(20).take(40) {
            assert!(row[..8].iter().all(|p| *p == BACKGROUND));
        }
        assert!(pixels[20 * 48..].iter().all(|p| *p == CONTENT));
    }

    #[test]
    fn tiny_and_empty_windows_are_safe() {
        let mut pixels = vec![0; 6];
        draw(&mut pixels, 3, 2.0);
        assert_eq!(pixels, vec![BACKGROUND; 6]);
        draw(&mut [], 0, 1.0);
    }
}
