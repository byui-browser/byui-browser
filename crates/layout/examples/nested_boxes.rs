use std::{fs, io::BufWriter, path::PathBuf};

use layout::{Rect as LayoutRect, create_rectangle};
use paint::{Color, DisplayItem, DisplayList, Rect as PaintRect};
use render::Compositor;

fn main() {
    let width = 640;
    let height = 360;
    let group_width = 280.0;
    let group_height = 312.0;
    let inset = 14.0;
    let border = 4.0;
    let ring_colors = [
        Color {
            r: 75,
            g: 20,
            b: 125,
            a: 255,
        },
        Color {
            r: 100,
            g: 35,
            b: 155,
            a: 255,
        },
        Color {
            r: 125,
            g: 55,
            b: 180,
            a: 255,
        },
        Color {
            r: 150,
            g: 80,
            b: 200,
            a: 255,
        },
        Color {
            r: 175,
            g: 105,
            b: 215,
            a: 255,
        },
        Color {
            r: 195,
            g: 135,
            b: 225,
            a: 255,
        },
        Color {
            r: 215,
            g: 165,
            b: 235,
            a: 255,
        },
    ];
    let text_color = Color {
        r: 55,
        g: 15,
        b: 90,
        a: 255,
    };

    let mut items = Vec::new();
    for (group_x, label) in [(24.0, "FFFFFFF"), (336.0, "RIGHTF")] {
        for (index, color) in ring_colors.iter().copied().enumerate() {
            let inset = inset * index as f32;
            let rect = create_rectangle(
                group_x + inset,
                24.0 + inset,
                group_width - 2.0 * inset,
                group_height - 2.0 * inset,
            );
            add_border(&mut items, rect, border, color);
        }

        let inner_x = group_x + inset * 6.0;
        let inner_y = 24.0 + inset * 6.0;
        let label_width = label.len() as f32 * 6.0;
        items.push(DisplayItem::Text {
            x: (inner_x + (group_width - 12.0 * inset - label_width) / 2.0).max(0.0),
            y: inner_y + (group_height - 12.0 * inset - 7.0) / 2.0,
            text: label.to_owned(),
            color: text_color,
        });
    }

    let frame = Compositor::new(width, height).compose_paint(&DisplayList { items });
    let output_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/nested-boxes.png");
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
