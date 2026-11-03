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
        .nodes
        .iter()
        .filter(|node| matches!(node.kind, html::NodeKind::Element(_)))
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
            .chunks_exact(4)
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
            paint::DisplayItem::FillRect { .. } => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(text_items, vec!["Hi"]);
}
