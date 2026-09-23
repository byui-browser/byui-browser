//! Rasterizer-facing drawing commands.

#[derive(Debug, Clone, PartialEq)]
pub enum DisplayItem {
    FillRect {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        rgba: [u8; 4],
    },
    Text {
        x: u32,
        y: u32,
        text: String,
        rgba: [u8; 4],
    },
}
