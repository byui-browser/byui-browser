//! Public-contract tests for `paint`.
//!
//! Run ignored tests with `cargo test -p paint -- --ignored` to see the backlog.

use paint::{Color, DisplayItem, PaintBox, Rect, build_display_list};

#[test]
#[ignore = "TODO(paint): background painting not implemented"]
fn box_with_background_yields_one_fill() {
    let red = Color {
        r: 255,
        g: 0,
        b: 0,
        a: 255,
    };
    let rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 10.0,
        height: 10.0,
    };
    let list = build_display_list(&[PaintBox {
        rect,
        background: Some(red),
    }]);
    assert_eq!(list.items, vec![DisplayItem::FillRect { rect, color: red }]);
}

#[test]
#[ignore = "TODO(paint): background painting not implemented"]
fn box_without_background_yields_nothing() {
    let list = build_display_list(&[PaintBox {
        rect: Rect::default(),
        background: None,
    }]);
    assert!(list.items.is_empty());
}
