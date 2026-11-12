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
/// Font family is carried through layout, but the current interface still uses
/// one font for all text. A font provider can use the family on
/// [`crate::TextFragment`] when selecting the font it passes to layout.
pub trait FontMetrics {
    /// Horizontal advance of `character` at `font_size` CSS pixels.
    fn advance(&self, character: char, font_size: f32) -> f32;

    /// Vertical metrics at `font_size` CSS pixels.
    fn vertical_metrics(&self, font_size: f32) -> VerticalMetrics;

    /// Horizontal advance using the requested CSS font-family fallback list.
    ///
    /// The default preserves compatibility with providers that expose one
    /// font only.
    fn advance_for_family(&self, character: char, font_size: f32, _family: &[String]) -> f32 {
        self.advance(character, font_size)
    }

    /// Vertical metrics using the requested CSS font-family fallback list.
    ///
    /// The default preserves compatibility with providers that expose one
    /// font only.
    fn vertical_metrics_for_family(&self, font_size: f32, _family: &[String]) -> VerticalMetrics {
        self.vertical_metrics(font_size)
    }

    /// Width of `text` at `font_size` CSS pixels, as the sum of advances.
    fn text_width(&self, text: &str, font_size: f32) -> f32 {
        text.chars()
            .map(|character| self.advance(character, font_size))
            .sum()
    }

    /// Width of `text` using the requested CSS font-family fallback list.
    fn text_width_for_family(&self, text: &str, font_size: f32, family: &[String]) -> f32 {
        text.chars()
            .map(|character| self.advance_for_family(character, font_size, family))
            .sum()
    }
}
