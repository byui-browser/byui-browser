//! First layout-team vertical slice: HTML -> layout -> paint -> pixels.

use std::fs;
use std::path::PathBuf;

use html::parse_raw_html;
use layout::{Size, StyledDom, layout_tree};
use paint::paint_document;
use render::Compositor;

fn write_ppm(path: &std::path::Path, frame: &render::Frame) {
    let mut ppm = format!("P6\n{} {}\n255\n", frame.width, frame.height).into_bytes();
    for rgba in frame.pixels.chunks_exact(4) {
        ppm.extend_from_slice(&rgba[..3]);
    }
    fs::write(path, ppm).expect("write rendered image");
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
    let layout = layout_tree(&StyledDom::from_html_document(&document), viewport);
    assert_eq!(layout.root.children.len(), 1);
    assert_eq!(layout.root.children[0].rect.width, viewport.width);

    let display_list = paint_document(&layout, &document);
    let frame =
        Compositor::new(viewport.width as u32, viewport.height as u32).compose_paint(&display_list);
    assert_eq!(&frame.pixels[0..4], &[210, 230, 255, 255]);
    assert!(
        frame
            .pixels
            .chunks_exact(4)
            .any(|pixel| pixel == [25, 45, 70, 255])
    );

    let output_dir = option_env!("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"));
    let output_dir = output_dir.join("layout-test-output");
    fs::create_dir_all(&output_dir).expect("create image output directory");
    let output_path = output_dir.join("simple-element.ppm");
    write_ppm(&output_path, &frame);
    assert!(output_path.is_file());

    eprintln!("wrote {}", output_path.display());
}

#[test]
fn nested_element_text_is_painted_once() {
    let document = parse_raw_html("<div><p>Hi</p></div>".to_owned());
    let layout = layout_tree(
        &StyledDom::from_html_document(&document),
        Size {
            width: 160.0,
            height: 120.0,
        },
    );

    let text_items = paint_document(&layout, &document)
        .items
        .into_iter()
        .filter_map(|item| match item {
            paint::DisplayItem::Text { text, .. } => Some(text),
            paint::DisplayItem::FillRect { .. } => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(text_items, vec!["Hi"]);
}
