//! Styles a synthetic, GitHub-sized page and reports parse and cascade time.
//!
//! Run with `cargo run -p css --release --example style_bench`.
//!
//! The generated stylesheet mimics a utility-class framework: thousands of
//! class rules, plus descendant, child, attribute, and pseudo-class selectors.
//! The document is a few thousand nested elements carrying those classes.

use std::fmt::Write as _;
use std::time::Instant;

const RULES: usize = 12_000;
const ELEMENTS: usize = 4_000;

fn stylesheet() -> String {
    let mut css = String::new();
    for index in 0..RULES {
        let class = format!("c{}", index % 3_000);
        let selector = match index % 10 {
            0 => format!(".{class}"),
            1 => format!(".{class}:hover"),
            2 => format!(".wrap .{class}"),
            3 => format!(".row > .{class}"),
            4 => format!("[data-mode=dark] .{class}"),
            5 => format!("div.{class}, span.{class}"),
            6 => format!(".{class}::before"),
            7 => format!(".{class}:not(.disabled)"),
            8 => format!("#id{index}"),
            _ => format!("a[href^=\"/{class}\"]"),
        };
        writeln!(
            css,
            "/* rule {index} */ {selector} {{ margin: {}px; color: #{:06x}; padding: 1px 2px }}",
            index % 7,
            index * 97 % 0xff_ffff
        )
        .unwrap();
    }
    css
}

fn document() -> String {
    let mut html = String::from("<html data-mode=dark><body><div class='wrap'>");
    let mut open = 0;
    for index in 0..ELEMENTS {
        let classes = format!("c{} c{} row", index * 7 % 3_000, index * 13 % 3_000);
        match index % 4 {
            0 => {
                write!(html, "<div class='{classes}' id='id{index}'>").unwrap();
                open += 1;
            }
            1 => write!(html, "<span class='{classes}'>text</span>").unwrap(),
            2 => write!(
                html,
                "<a href='/c{}' class='{classes}'>link</a>",
                index % 3_000
            )
            .unwrap(),
            _ => {
                if open > 0 {
                    html.push_str("</div>");
                    open -= 1;
                }
            }
        }
    }
    html.push_str("</div></body></html>");
    html
}

fn main() {
    let css = stylesheet();
    let html = document();
    let dom = html::parse_raw_html(html);

    let start = Instant::now();
    let sheet = css::parse_stylesheet(&css);
    let parse = start.elapsed();

    let start = Instant::now();
    let styles = css::style_document(&dom, std::slice::from_ref(&sheet));
    let cascade = start.elapsed();

    println!(
        "stylesheet: {} bytes, {} rules; document: {} nodes",
        css.len(),
        sheet.rules.len(),
        dom.nodes.len()
    );
    println!("parse:   {parse:?}");
    println!(
        "cascade: {cascade:?} ({} styled elements)",
        styles.styles.len()
    );
}
