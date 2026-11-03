//! Public-contract tests for `layout`.

use layout::{FontMetrics, Size, VerticalMetrics, layout_document};

/// A fixed-pitch font: every character is half an em wide.
struct TestFont;

impl FontMetrics for TestFont {
    fn advance(&self, _character: char, font_size: f32) -> f32 {
        font_size / 2.0
    }

    fn vertical_metrics(&self, font_size: f32) -> VerticalMetrics {
        VerticalMetrics {
            ascent: font_size * 0.8,
            descent: font_size * 0.2,
            line_gap: font_size * 0.2,
        }
    }
}

#[test]
fn block_children_stack_vertically() {
    let document = html::parse_raw_html("<div><p>One</p><p>Two</p></div>".into());
    let styles = css::style_document(&document, &[]);
    let tree = layout_document(
        &document,
        &styles,
        Size {
            width: 100.0,
            height: 100.0,
        },
        &TestFont,
    );
    let div = &tree.root.children[0];
    let [first, second] = div.children.as_slice() else {
        panic!("expected two block children");
    };
    // The div is the root element, which establishes a block formatting
    // context, so the first paragraph's 1em top margin stays inside it.
    assert_eq!(first.rect.y, 16.0);
    // Adjacent 1em margins collapse to one 16px gap instead of adding up.
    assert_eq!(second.rect.y, first.rect.y + first.rect.height + 16.0);
}
