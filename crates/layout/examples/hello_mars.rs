use std::{fs, io::BufWriter, path::PathBuf};

use layout::{Rect as LayoutRect, create_rectangle};
use paint::{Color, DisplayItem, DisplayList, Rect as PaintRect};
use render::Compositor;

fn main() {
    let width = 320;
    let height = 180;
    let box_rect = create_rectangle(0.0, 0.0, width as f32, height as f32);
    let border = 12.0;
    let fill = Color {
        r: 190,
        g: 130,
        b: 245,
        a: 255,
    };
    let border_color = Color {
        r: 90,
        g: 25,
        b: 145,
        a: 255,
    };
    let text_color = Color {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };

    let mut items = vec![DisplayItem::FillRect {
        rect: as_paint_rect(box_rect),
        color: fill,
    }];
    add_border(&mut items, box_rect, border, border_color);
    items.push(DisplayItem::Text {
        x: box_rect.x + 89.0,
        y: box_rect.y + 49.0,
        text: "Hello Mars, We Are Earthlings".to_owned(),
        color: text_color,
    });

    let frame = Compositor::new(width, height).compose_paint(&DisplayList { items });
    let output_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/hello-mars.png");
    fs::create_dir_all(output_path.parent().expect("output path has a parent"))
        .expect("create output directory");
    let file = fs::File::create(&output_path).expect("create PNG file");
    let mut encoder = png::Encoder::new(BufWriter::new(file), frame.width, frame.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .expect("write PNG header")
        .write_image_data(&frame.pixels)
        .expect("write PNG pixels");
    println!("Wrote {}", output_path.display());
}

fn add_border(items: &mut Vec<DisplayItem>, rect: LayoutRect, thickness: f32, color: Color) {
    let bands = [
        create_rectangle(rect.x, rect.y, rect.width, thickness),
        create_rectangle(
            rect.x,
            rect.y + rect.height - thickness,
            rect.width,
            thickness,
        ),
        create_rectangle(
            rect.x,
            rect.y + thickness,
            thickness,
            rect.height - 2.0 * thickness,
        ),
        create_rectangle(
            rect.x + rect.width - thickness,
            rect.y + thickness,
            thickness,
            rect.height - 2.0 * thickness,
        ),
    ];
    items.extend(bands.into_iter().map(|rect| DisplayItem::FillRect {
        rect: as_paint_rect(rect),
        color,
    }));
}

fn as_paint_rect(rect: LayoutRect) -> PaintRect {
    PaintRect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}
