//! First layout-team vertical slice: HTML -> layout -> paint -> pixels.

use std::fs;
use std::path::PathBuf;

use html::parse_raw_html;
use layout::{Size, layout_document};
use paint::paint_document;
use render::{BitmapFont, Compositor};

fn write_png(path: &std::path::Path, frame: &render::Frame) {
    let file = fs::File::create(path).expect("create rendered image");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), frame.width, frame.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .expect("write png header")
        .write_image_data(&frame.pixels)
        .expect("write png pixels");
}

#[test]
fn parses_lays_out_paints_and_writes_a_simple_element_image() {
    let document = parse_raw_html("<div>Hello, browser!</div>".to_owned());
    let element_count = document
        .get_nodes()
        .iter()
        .filter(|node| matches!(node.get_kind(), html::NodeKind::Element(_)))
        .count();
    assert_eq!(element_count, 1);

    let viewport = Size {
        width: 160.0,
        height: 120.0,
    };
    let styles = css::style_document(&document, &[]);
    let layout = layout_document(&document, &styles, viewport, &BitmapFont);
    assert_eq!(layout.root.children.len(), 1);
    assert_eq!(layout.root.children[0].rect.width, viewport.width);

    let display_list = paint_document(&layout);
    let frame =
        Compositor::new(viewport.width as u32, viewport.height as u32).compose_paint(&display_list);
    assert_eq!(&frame.pixels[0..4], &[255, 255, 255, 255]);
    assert!(
        frame
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .copied()
            .any(|pixel| pixel == [0, 0, 0, 255])
    );

    let output_dir = option_env!("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"));
    let output_dir = output_dir.join("layout-test-output");
    fs::create_dir_all(&output_dir).expect("create image output directory");
    let output_path = output_dir.join("simple-element.png");
    write_png(&output_path, &frame);
    assert!(output_path.is_file());

    eprintln!("wrote {}", output_path.display());
}

#[test]
fn nested_element_text_is_painted_once() {
    let document = parse_raw_html("<div><p>Hi</p></div>".to_owned());
    let styles = css::style_document(&document, &[]);
    let layout = layout_document(
        &document,
        &styles,
        Size {
            width: 160.0,
            height: 120.0,
        },
        &BitmapFont,
    );

    let text_items = paint_document(&layout)
        .items
        .into_iter()
        .filter_map(|item| match item {
            paint::DisplayItem::Text { text, .. } => Some(text),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(text_items, vec!["Hi"]);
}

#[test]
fn css_borders_size_boxes_and_reach_screen_pixels() {
    let document = parse_raw_html("<html><body><div></div></body></html>".to_owned());
    let styles = css::style_document(
        &document,
        &[css::parse_stylesheet(
            "body { margin: 0 } div { width: 30px; height: 24px; box-sizing: border-box;
         background: white; background-color: white; color: red; border: 2px solid;
         border-top: 3px solid blue; border-right-color: green;
         border-bottom: 4px solid black; border-left: 5px solid red; }",
        )],
    );
    let tree = layout_document(
        &document,
        &styles,
        Size {
            width: 50.0,
            height: 50.0,
        },
        &BitmapFont,
    );
    let list = paint::paint_document(&tree);
    let frame = render::Compositor::new(50, 50).compose_paint(&list);
    let pixel = |x: usize, y: usize| &frame.pixels[(y * 50 + x) * 4..(y * 50 + x) * 4 + 4];
    assert_eq!(pixel(15, 1), [0, 0, 255, 255]);
    assert_eq!(pixel(29, 10), [0, 128, 0, 255]);
    assert_eq!(pixel(15, 23), [0, 0, 0, 255]);
    assert_eq!(pixel(2, 10), [255, 0, 0, 255]);
    assert_eq!(pixel(15, 10), [255, 255, 255, 255]);
    assert_eq!(pixel(31, 10), [255, 255, 255, 255]);
    let element = tree
        .root
        .children
        .iter()
        .flat_map(|b| &b.children)
        .flat_map(|b| &b.children)
        .next()
        .unwrap();
    assert_eq!(element.rect.width, 30.0);
    assert_eq!(element.content_rect().width, 23.0);
    assert_eq!(element.content_rect().height, 17.0);
}
