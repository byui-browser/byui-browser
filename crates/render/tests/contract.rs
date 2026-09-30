//! Public-contract tests for `render`.
//!
//! Run ignored tests with `cargo test -p render -- --ignored` to see the backlog.

use render::{Compositor, DisplayItem};

#[test]
#[ignore = "TODO(render): rasterization not implemented"]
fn fill_rect_writes_pixels_inside_and_leaves_outside_cleared() {
    let frame = Compositor::new(4, 4).compose(&[DisplayItem::FillRect {
        x: 0,
        y: 0,
        width: 2,
        height: 2,
        rgba: [255, 0, 0, 255],
    }]);
    let px = |x: usize, y: usize| &frame.pixels[(y * 4 + x) * 4..(y * 4 + x) * 4 + 4];
    assert_eq!(px(0, 0), [255, 0, 0, 255]);
    assert_eq!(px(3, 3), Compositor::CLEAR_COLOR);
}
