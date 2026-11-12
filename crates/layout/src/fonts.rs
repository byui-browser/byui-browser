//! The font measurement interface layout uses to size text.

/// Vertical font metrics at one font size, in CSS pixels.
///
/// `ascent` and `descent` are the distances above and below the alphabetic
/// baseline that bound the font's glyphs; together they are the height of an
/// inline box's content area (CSS 2.2 §10.6.1). `line_gap` is the extra
/// leading the font recommends, which `line-height: normal` adds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VerticalMetrics {
    /// Distance from the baseline up to the top of the content area.
    pub ascent: f32,
    /// Distance from the baseline down to the bottom of the content area.
    pub descent: f32,
    /// Recommended extra space between lines.
    pub line_gap: f32,
}

/// Measures text for layout.
///
/// Layout never rasterizes glyphs. Instead the embedder supplies the font that
/// will draw the text, so that line breaking and the painted glyphs agree.
/// `render::BitmapFont` implements this for the software renderer.
///
/// Font family, weight, and style are not modelled yet: one font is used for
/// all text, and only `font-size` varies.
pub trait FontMetrics {
    /// Horizontal advance of `character` at `font_size` CSS pixels.
    fn advance(&self, character: char, font_size: f32) -> f32;

    /// Vertical metrics at `font_size` CSS pixels.
    fn vertical_metrics(&self, font_size: f32) -> VerticalMetrics;

    /// Width of `text` at `font_size` CSS pixels, as the sum of advances.
    fn text_width(&self, text: &str, font_size: f32) -> f32 {
        text.chars()
            .map(|character| self.advance(character, font_size))
            .sum()
    }
}
