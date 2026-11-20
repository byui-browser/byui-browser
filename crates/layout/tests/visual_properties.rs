//! End-to-end rendering tests for visual CSS properties: HTML and CSS in,
//! pixels out, through layout, paint, and the software compositor.
//!
//! Covers `opacity`, `border-color`, `font`, `font-weight`, `font-style`,
//! `overflow`, `z-index`, and `transform` with its `-webkit-` and `-ms-`
//! aliases. Text uses the built-in bitmap font.

use html::{HTMLDocument, NodeID};
use layout::{LayoutTree, LineFragment, PaintItem, Size, TextFragment, layout_document};
use render::{BitmapFont, Compositor, Frame};

const WIDTH: u32 = 100;
const HEIGHT: u32 = 100;

fn lay_out(html: &str, css: &str) -> (HTMLDocument, LayoutTree) {
    let document = html::parse_raw_html(format!("<html><body>{html}</body></html>"));
    let css = format!("body {{ margin: 0 }} {css}");
    let styles = css::style_document(&document, &[css::parse_stylesheet(&css)]);
    let tree = layout_document(
        &document,
        &styles,
        Size {
            width: WIDTH as f32,
            height: HEIGHT as f32,
        },
        &BitmapFont,
    );
    (document, tree)
}

fn render(html: &str, css: &str) -> Frame {
    let (_, tree) = lay_out(html, css);
    Compositor::new(WIDTH, HEIGHT).compose_paint(&paint::paint_document(&tree))
}

fn pixel(frame: &Frame, x: u32, y: u32) -> [u8; 4] {
    let offset = ((y * frame.width + x) * 4) as usize;
    frame.pixels[offset..offset + 4].try_into().unwrap()
}

/// Asserts each channel of the pixel at `(x, y)` is within 2 of `expected`.
#[track_caller]
fn assert_pixel(frame: &Frame, x: u32, y: u32, expected: [u8; 3]) {
    let actual = pixel(frame, x, y);
    assert!(
        actual.iter().zip(expected).all(|(a, e)| a.abs_diff(e) <= 2),
        "pixel ({x}, {y}) is {actual:?}, expected about {expected:?}"
    );
}

const WHITE: [u8; 3] = [255, 255, 255];
const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 128, 0];
const BLUE: [u8; 3] = [0, 0, 255];

fn id(document: &HTMLDocument, id: &str) -> NodeID {
    document.get_element_by_id(id).expect("element with id")
}

fn texts(tree: &LayoutTree) -> Vec<&TextFragment> {
    tree.paint_order()
        .into_iter()
        .filter_map(|item| match item {
            PaintItem::Text(text) if !text.text.trim().is_empty() => Some(text),
            _ => None,
        })
        .collect()
}

/// How many pixels differ from white.
fn ink(frame: &Frame) -> usize {
    frame
        .pixels
        .chunks(4)
        .filter(|pixel| pixel[..3] != [255, 255, 255])
        .count()
}

#[test]
fn opacity_blends_a_box_over_what_is_below() {
    let frame = render(
        "<div></div>",
        "div { width: 20px; height: 20px; background-color: red; opacity: 0.5 }",
    );
    assert_pixel(&frame, 10, 10, [255, 128, 128]);
    assert_pixel(&frame, 30, 10, WHITE);
    let percent = render(
        "<div></div>",
        "div { width: 20px; height: 20px; background-color: blue; opacity: 25% }",
    );
    assert_pixel(&percent, 10, 10, [191, 191, 255]);
    let invisible = render(
        "<div>Hello</div>",
        "div { height: 20px; background-color: red; opacity: 0 }",
    );
    assert_eq!(ink(&invisible), 0);
}

#[test]
fn opacity_fades_a_box_and_its_descendants_as_one_group() {
    // A group fades the composite: the blue child hides the red parent
    // completely, then the whole is blended, so no red shows through.
    let frame = render(
        "<div id=parent><div id=child></div></div>",
        "#parent { width: 20px; height: 20px; background-color: red; opacity: 0.5 }
         #child { width: 20px; height: 20px; background-color: blue }",
    );
    assert_pixel(&frame, 10, 10, [128, 128, 255]);
    // Nested groups multiply.
    let nested = render(
        "<div id=outer><div id=inner></div></div>",
        "#outer { opacity: 0.5 }
         #inner { width: 20px; height: 20px; background-color: blue; opacity: 0.5 }",
    );
    assert_pixel(&nested, 10, 10, [191, 191, 255]);
}

#[test]
fn opacity_on_inline_boxes_fades_their_colors_and_text() {
    let (_, tree) = lay_out(
        "<p><span>A<b>B</b></span>C</p>",
        "p { margin: 0 } span { opacity: 0.5; background-color: blue } b { opacity: 0.5 }",
    );
    let alphas: Vec<(&str, u8)> = texts(&tree)
        .into_iter()
        .map(|text| (text.text.as_str(), text.color.a))
        .collect();
    assert_eq!(alphas, [("A", 128), ("B", 64), ("C", 255)]);
    let spans: Vec<u8> = tree
        .paint_order()
        .into_iter()
        .filter_map(|item| match item {
            PaintItem::InlineBox(inline) => inline.style.background_color.map(|color| color.a),
            _ => None,
        })
        .collect();
    assert_eq!(spans, [128]);
}

#[test]
fn border_color_sets_each_side_and_transparent_shows_the_background() {
    let frame = render(
        "<div></div>",
        "div { width: 20px; height: 20px; border: 4px solid; border-color: red green blue black }",
    );
    assert_pixel(&frame, 14, 1, RED);
    assert_pixel(&frame, 26, 14, GREEN);
    assert_pixel(&frame, 14, 26, BLUE);
    assert_pixel(&frame, 1, 14, [0, 0, 0]);
    let transparent = render(
        "<div></div>",
        "div { width: 20px; height: 20px; border: 4px solid red; border-top-color: transparent;
               color: blue; border-left-color: currentcolor }",
    );
    assert_pixel(&transparent, 14, 1, WHITE);
    assert_pixel(&transparent, 1, 14, BLUE);
    assert_pixel(&transparent, 26, 14, RED);
}

#[test]
fn font_shorthand_sets_size_line_height_weight_and_style() {
    let (_, tree) = lay_out(
        "<p>Hi</p>",
        "p { margin: 0; font: italic bold 20px/30px monospace }",
    );
    let text = texts(&tree)[0];
    assert_eq!(text.font_size, 20.0);
    assert!(text.synthetic_bold);
    assert!(text.synthetic_italic);
    let paragraph = &tree.root.children[0].children[0].children[0];
    assert_eq!(paragraph.rect.height, 30.0);
    // Omitted components reset: a later shorthand clears the bold.
    let (_, reset) = lay_out(
        "<p>Hi</p>",
        "p { margin: 0; font-weight: bold; font: 12px serif }",
    );
    let text = texts(&reset)[0];
    assert_eq!(text.font_size, 12.0);
    assert!(!text.synthetic_bold);
    assert!(!text.synthetic_italic);
}

#[test]
fn font_weight_and_font_style_render_synthetic_bold_and_italic() {
    let css = |extra: &str| format!("p {{ margin: 0; font-size: 20px; {extra} }}");
    let regular = render("<p>HI</p>", &css(""));
    let bold = render("<p>HI</p>", &css("font-weight: bold"));
    let numeric = render("<p>HI</p>", &css("font-weight: 700"));
    let light = render("<p>HI</p>", &css("font-weight: 500"));
    assert!(ink(&bold) > ink(&regular), "bold text should be heavier");
    assert_eq!(bold, numeric);
    assert_eq!(light, regular);

    let italic = render("<p>HI</p>", &css("font-style: italic"));
    let oblique = render("<p>HI</p>", &css("font-style: oblique"));
    assert_ne!(italic, regular);
    assert_eq!(italic, oblique);
    // The slant leans right: the top row's ink starts further right.
    let first_ink =
        |frame: &Frame, y: u32| (0..WIDTH).find(|&x| pixel(frame, x, y)[..3] != [255, 255, 255]);
    let top = (0..HEIGHT)
        .find(|&y| first_ink(&regular, y).is_some())
        .unwrap();
    assert!(first_ink(&italic, top).unwrap() > first_ink(&regular, top).unwrap());
}

#[test]
fn overflow_hidden_clips_descendants_to_the_padding_box() {
    let html = "<div id=clip><div id=big></div></div>";
    let css = "#clip { width: 20px; height: 20px; border: 2px solid blue; overflow: hidden }
               #big { width: 60px; height: 60px; background-color: red }";
    let frame = render(html, css);
    assert_pixel(&frame, 10, 10, RED);
    assert_pixel(&frame, 1, 10, BLUE);
    assert_pixel(&frame, 40, 40, WHITE);
    assert_pixel(&frame, 30, 10, WHITE);
    // Unclipped, the child overflows.
    let visible = render(html, &css.replace("overflow: hidden", "overflow: visible"));
    assert_pixel(&visible, 40, 40, RED);
    // Hit testing matches: the clipped-away part of the child misses it.
    let (document, tree) = lay_out(html, css);
    assert_eq!(tree.hit_test(10.0, 10.0), Some(id(&document, "big")));
    assert_ne!(tree.hit_test(40.0, 40.0), Some(id(&document, "big")));
}

#[test]
fn overflow_clips_one_axis_and_spares_escaping_positioned_descendants() {
    let frame = render(
        "<div id=clip><div id=big></div></div>",
        "#clip { width: 20px; height: 20px; overflow-x: clip }
         #big { width: 60px; height: 60px; background-color: red }",
    );
    assert_pixel(&frame, 10, 40, RED);
    assert_pixel(&frame, 40, 10, WHITE);
    // An absolutely positioned child whose containing block is outside
    // the clipping box is not clipped; one inside it is.
    let html = "<div id=clip><div id=abs></div></div>";
    let css = "#clip { width: 20px; height: 20px; overflow: hidden }
               #abs { position: absolute; top: 0; left: 0; width: 60px; height: 60px;
                      background-color: red }";
    assert_pixel(&render(html, css), 40, 40, RED);
    let contained = css.replace("overflow: hidden", "overflow: hidden; position: relative");
    assert_pixel(&render(html, &contained), 40, 40, WHITE);
}

#[test]
fn z_index_orders_positioned_boxes() {
    let html = "<div id=a></div><div id=b></div>";
    let css = "div { position: absolute; top: 0; left: 0; width: 20px; height: 20px }
               #a { background-color: red; z-index: 2 } #b { background-color: blue; z-index: 1 }";
    let frame = render(html, css);
    assert_pixel(&frame, 10, 10, RED);
    // Without z-index, tree order decides.
    let auto = render(
        html,
        &css.replace("z-index: 2", "").replace("z-index: 1", ""),
    );
    assert_pixel(&auto, 10, 10, BLUE);
    let (document, tree) = lay_out(html, css);
    assert_eq!(tree.hit_test(10.0, 10.0), Some(id(&document, "a")));
}

#[test]
fn negative_z_index_paints_below_in_flow_blocks() {
    let frame = render(
        "<div id=flow></div><div id=behind></div>",
        "#flow { width: 20px; height: 20px; background-color: green }
         #behind { position: absolute; top: 10px; left: 10px; width: 20px; height: 20px;
                   background-color: red; z-index: -1 }",
    );
    assert_pixel(&frame, 15, 15, GREEN);
    assert_pixel(&frame, 25, 25, RED);
}

#[test]
fn z_index_compares_across_positioned_ancestors_without_z_index() {
    // `#wrap` is positioned but has no z-index, so it creates no stacking
    // context: its child's z-index 10 competes with `#cover`'s 5.
    let frame = render(
        "<div id=wrap><div id=top></div></div><div id=cover></div>",
        "#wrap { position: relative }
         #top { position: absolute; width: 20px; height: 20px; background-color: red; z-index: 10 }
         #cover { position: absolute; top: 0; width: 20px; height: 20px; background-color: blue;
                  z-index: 5 }",
    );
    assert_pixel(&frame, 10, 10, RED);
    // A z-index on the wrapper traps the child inside its stacking context.
    let trapped = render(
        "<div id=wrap><div id=top></div></div><div id=cover></div>",
        "#wrap { position: relative; z-index: 1 }
         #top { position: absolute; width: 20px; height: 20px; background-color: red; z-index: 10 }
         #cover { position: absolute; top: 0; width: 20px; height: 20px; background-color: blue;
                  z-index: 5 }",
    );
    assert_pixel(&trapped, 10, 10, BLUE);
}

#[test]
fn z_index_applies_to_flex_items_without_positioning() {
    let frame = render(
        "<div id=flex><div id=a></div><div id=b></div></div>",
        "#flex { display: flex }
         #a { width: 20px; height: 20px; background-color: red; z-index: 1 }
         #b { width: 20px; height: 20px; margin-left: -10px; background-color: blue }",
    );
    assert_pixel(&frame, 15, 10, RED);
}

#[test]
fn transform_moves_scales_and_rotates_without_affecting_layout() {
    let css = |transform: &str| {
        format!(
            "#t {{ width: 20px; height: 20px; background-color: red; {transform} }}
             #next {{ height: 10px; background-color: blue }}"
        )
    };
    let html = "<div id=t></div><div id=next></div>";
    let moved = render(html, &css("transform: translate(30px, 10px)"));
    assert_pixel(&moved, 40, 20, RED);
    assert_pixel(&moved, 5, 5, WHITE);
    // The next block stays where layout put it.
    assert_pixel(&moved, 80, 25, BLUE);

    let scaled = render(
        html,
        &css("transform: scale(2); transform-origin: left top"),
    );
    assert_pixel(&scaled, 35, 35, RED);
    assert_pixel(&scaled, 45, 45, WHITE);

    // A 45° turn about the center puts a corner of the square straight
    // above the center, 14px up; the old corners are now empty.
    let rotated = render(html, &css("transform: rotate(45deg)"));
    assert_pixel(&rotated, 10, 0, RED);
    assert_pixel(&rotated, 1, 1, WHITE);
    assert_pixel(&rotated, 10, 10, RED);

    let percent = render(html, &css("transform: translateX(100%)"));
    assert_pixel(&percent, 30, 10, RED);
    assert_pixel(&percent, 10, 10, WHITE);
}

#[test]
fn prefixed_transforms_render_like_transform() {
    let html = "<div id=t></div>";
    let base = "#t { width: 20px; height: 20px; background-color: red; ";
    let standard = render(html, &format!("{base} transform: translate(30px, 10px) }}"));
    for prefixed in ["-webkit-transform", "-ms-transform"] {
        let frame = render(
            html,
            &format!("{base} {prefixed}: translate(30px, 10px) }}"),
        );
        assert_eq!(frame, standard, "{prefixed}");
    }
    let origin = render(
        html,
        &format!("{base} -webkit-transform: scale(2); -webkit-transform-origin: 0 0 }}"),
    );
    assert_pixel(&origin, 35, 35, RED);
}

#[test]
fn transformed_boxes_hit_test_where_they_are_drawn() {
    let (document, tree) = lay_out(
        "<div id=t></div>",
        "#t { width: 20px; height: 20px; transform: translate(50px, 50px) }",
    );
    assert_eq!(tree.hit_test(60.0, 60.0), Some(id(&document, "t")));
    assert_ne!(tree.hit_test(10.0, 10.0), Some(id(&document, "t")));
}

#[test]
fn transforms_and_opacity_create_layers_in_paint_order() {
    let (_, tree) = lay_out(
        "<div id=t>x</div>",
        "#t { transform: rotate(10deg); opacity: 0.5; overflow: hidden; height: 10px }",
    );
    let markers: Vec<&str> = tree
        .paint_order()
        .into_iter()
        .filter_map(|item| match item {
            PaintItem::BeginLayer(_) => Some("begin"),
            PaintItem::EndLayer(_) => Some("end"),
            PaintItem::Clip(Some(_)) => Some("clip"),
            PaintItem::Clip(None) => Some("unclip"),
            _ => None,
        })
        .collect();
    assert_eq!(markers, ["begin", "clip", "end"]);
    let text = tree.root.children[0].children[0].children[0]
        .lines
        .iter()
        .flat_map(|line| &line.fragments)
        .any(|fragment| matches!(fragment, LineFragment::Text(_)));
    assert!(text);
}

#[test]
fn transformed_content_scrolled_into_view_is_painted() {
    // The box is laid out below the viewport but translated up into it;
    // painting must not cull it as off-screen.
    let (_, tree) = lay_out(
        "<div id=spacer></div><div id=t></div>",
        "#spacer { height: 300px } #t { width: 20px; height: 20px; background-color: red;
                                       transform: translateY(-280px) }",
    );
    let list = paint::paint_document_scrolled(&tree, 0.0);
    let frame = Compositor::new(WIDTH, HEIGHT).compose_paint(&list);
    assert_pixel(&frame, 10, 30, RED);
}
