//! Practice: build a paint list by hand and render a colored box to a PNG.

use std::fs;
use std::path::PathBuf;

use paint::{Color, DisplayItem, DisplayList, Rect};
use render::Compositor;

fn write_png(path: &std::path::Path, frame: &render::Frame) {
    let file = fs::File::create(path).expect("create png file");
    let mut encoder =
        png::Encoder::new(std::io::BufWriter::new(file), frame.width, frame.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .expect("write png header")
        .write_image_data(&frame.pixels)
        .expect("write png pixels");
}

#[test]
fn draws_a_red_box_to_png() {
    // --- Step A: describe what to paint ---
    // A DisplayList is just an ordered list of drawing commands.
    // We add one FillRect command: a red 60x40 box at position (20, 15).
    let mut list = DisplayList::default();
    list.items.push(DisplayItem::FillRect {
        rect: Rect {
            x: 20.0,
            y: 15.0,
            width: 60.0,
            height: 40.0,
        },
        color: Color {
            r: 220,
            g: 50,
            b: 50,
            a: 255,
        },
    });

    // --- Step B: rasterize to pixels ---
    // Compositor takes the canvas size (width, height in device pixels).
    // compose_paint converts CSS-pixel f32 coords → device pixels and fills them in.
    let frame = Compositor::new(120, 80).compose_paint(&list);

    // --- Step C: verify a pixel inside the box is the color we asked for ---
    // Pixel at (20, 15) = row 15, col 20 → byte offset (15 * 120 + 20) * 4
    let idx = (15 * 120 + 20) * 4;
    assert_eq!(
        &frame.pixels[idx..idx + 4],
        &[220, 50, 50, 255],
        "expected red pixel inside the box"
    );

    // --- Step D: write the PNG so you can open it and see it ---
    let output_dir = option_env!("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"));
    let output_dir = output_dir.join("layout-test-output");
    fs::create_dir_all(&output_dir).expect("create output dir");
    let output_path = output_dir.join("draw-box.png");
    write_png(&output_path, &frame);
    eprintln!("wrote {}", output_path.display());
}
