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

/// Identifies one font face chosen by [`FontMetrics::select_font`].
///
/// The number is meaningful only to the [`FontMetrics`] that returned it;
/// layout stores it on text fragments and paint passes it to the renderer,
/// which must use a font collection that agrees with the one layout measured
/// with.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct FontId(pub u32);

impl FontId {
    /// The embedder's default font, used when no family matches and by
    /// [`FontMetrics`] implementations with a single font.
    pub const DEFAULT: Self = Self(0);
}

/// Measures text for layout.
///
/// Layout never rasterizes glyphs. Instead the embedder supplies the font that
/// will draw the text, so that line breaking and the painted glyphs agree.
/// `render::BitmapFont` and `render::OutlineFont` implement this with a
/// single font; `render::FontCollection` adds `@font-face` web fonts.
///
/// Layout calls [`Self::select_font`] once per element style to choose a face
/// from the computed `font-family`, `font-weight`, and `font-style`, then
/// measures with the `font_*` methods. Their default implementations ignore
/// the font and forward to the single-font methods, so a single-font
/// implementation only needs [`Self::advance`] and [`Self::vertical_metrics`].
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

    /// Chooses the face for a computed `font-family` list (names in order
    /// of preference, unquoted), `font-weight` (1–1000), and whether
    /// `font-style` is italic or oblique. Returns [`FontId::DEFAULT`] when
    /// nothing matches; the default implementation always does.
    fn select_font(&self, families: &[String], weight: u16, italic: bool) -> FontId {
        let _ = (families, weight, italic);
        FontId::DEFAULT
    }

    /// Horizontal advance of `character` in `font` at `font_size` CSS
    /// pixels.
    fn font_advance(&self, font: FontId, character: char, font_size: f32) -> f32 {
        let _ = font;
        self.advance(character, font_size)
    }

    /// Vertical metrics of `font` at `font_size` CSS pixels.
    fn font_vertical_metrics(&self, font: FontId, font_size: f32) -> VerticalMetrics {
        let _ = font;
        self.vertical_metrics(font_size)
    }

    /// Width of `text` in `font` at `font_size` CSS pixels, as the sum of
    /// advances.
    fn font_text_width(&self, font: FontId, text: &str, font_size: f32) -> f32 {
        text.chars()
            .map(|character| self.font_advance(font, character, font_size))
            .sum()
    }
}
