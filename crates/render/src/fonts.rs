//! Small built-in bitmap font for the software-rendered first slice.

/// The built-in 5×7 bitmap font, described in font units.
///
/// The em square is [`BitmapFont::UNITS_PER_EM`] units tall. Every character
/// advances [`BitmapFont::ADVANCE`] units, including characters without a
/// glyph, which draw nothing. Glyph rows occupy the 7 units directly above the
/// baseline. Glyphs scale with `font-size`: at 10px one unit is one CSS pixel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BitmapFont;

impl BitmapFont {
    /// Font units per em.
    pub const UNITS_PER_EM: f32 = 10.0;
    /// Horizontal advance of every character, in font units.
    pub const ADVANCE: f32 = 6.0;
    /// Distance from the baseline to the top of the em box, in font units.
    pub const ASCENT: f32 = 8.0;
    /// Distance from the baseline to the bottom of the em box, in font units.
    pub const DESCENT: f32 = 2.0;
    /// Extra leading for `line-height: normal`, in font units, giving a normal
    /// line height of 1.2em.
    pub const LINE_GAP: f32 = 2.0;
    /// Glyph bitmap width in font units.
    pub const GLYPH_WIDTH: u32 = 5;
    /// Glyph bitmap height in font units.
    pub const GLYPH_HEIGHT: u32 = 7;

    /// CSS pixels per font unit at `font_size` CSS pixels.
    pub fn scale(font_size: f32) -> f32 {
        font_size / Self::UNITS_PER_EM
    }
}

impl layout::FontMetrics for BitmapFont {
    fn advance(&self, _character: char, font_size: f32) -> f32 {
        Self::ADVANCE * Self::scale(font_size)
    }

    fn vertical_metrics(&self, font_size: f32) -> layout::VerticalMetrics {
        let scale = Self::scale(font_size);
        layout::VerticalMetrics {
            ascent: Self::ASCENT * scale,
            descent: Self::DESCENT * scale,
            line_gap: Self::LINE_GAP * scale,
        }
    }
}

/// A scalable outline font (TrueType or OpenType) for page text.
///
/// Sizes are em sizes in CSS pixels, as `font-size` specifies them, so text
/// set at 16px has a 16px em square. Layout measures with this font through
/// [`layout::FontMetrics`], and [`crate::Compositor::compose_paint_with_font`]
/// draws with the same advances, so line breaks and painted glyphs agree.
///
/// Limitation: glyphs are mapped one per character and advanced without
/// kerning or shaping, so ligatures and complex scripts are not supported.
/// Characters the font has no glyph for draw the font's missing-glyph shape.
#[derive(Debug, Clone)]
pub struct OutlineFont {
    font: ab_glyph::FontArc,
    /// Font units per em; converts em sizes to the font's design units.
    units_per_em: f32,
}

impl OutlineFont {
    /// Parses font file `data`, using the first face of a font collection.
    ///
    /// Returns `None` when the data is not a supported font.
    pub fn from_bytes(data: Vec<u8>) -> Option<Self> {
        let font = ab_glyph::FontArc::try_from_vec(data).ok()?;
        let units_per_em = ab_glyph::Font::units_per_em(&font)?;
        Some(Self { font, units_per_em })
    }

    /// Loads the first of `paths` that reads and parses as a font, or `None`
    /// when none does.
    pub fn load_first<P: AsRef<std::path::Path>>(
        paths: impl IntoIterator<Item = P>,
    ) -> Option<Self> {
        paths
            .into_iter()
            .find_map(|path| Self::from_bytes(std::fs::read(path).ok()?))
    }

    /// Design units to CSS pixels at an em size of `font_size` CSS pixels.
    fn units_to_px(&self, font_size: f32) -> f32 {
        font_size / self.units_per_em
    }

    /// The `ab_glyph` scale for an em size of `font_size` pixels. `PxScale`
    /// measures ascent-to-descent height rather than the em square.
    pub(crate) fn px_scale(&self, font_size: f32) -> ab_glyph::PxScale {
        ab_glyph::PxScale::from(
            ab_glyph::Font::height_unscaled(&self.font) * self.units_to_px(font_size),
        )
    }

    /// The parsed font, for rasterizing.
    pub(crate) fn font(&self) -> &ab_glyph::FontArc {
        &self.font
    }
}

impl layout::FontMetrics for OutlineFont {
    fn advance(&self, character: char, font_size: f32) -> f32 {
        use ab_glyph::Font;
        let id = self.font.glyph_id(character);
        self.font.h_advance_unscaled(id) * self.units_to_px(font_size)
    }

    fn vertical_metrics(&self, font_size: f32) -> layout::VerticalMetrics {
        use ab_glyph::Font;
        let scale = self.units_to_px(font_size);
        layout::VerticalMetrics {
            ascent: self.font.ascent_unscaled() * scale,
            // `ab_glyph` reports descent as a negative offset below the baseline.
            descent: -self.font.descent_unscaled() * scale,
            line_gap: self.font.line_gap_unscaled() * scale,
        }
    }
}

/// Returns the seven-row, five-column bitmap glyph for a supported character.
///
/// The first slice supports a small bitmap font; unsupported
/// characters are skipped by the compositor.
pub fn glyph(character: char) -> Option<[u8; 7]> {
    let glyph = match character.to_ascii_uppercase() {
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'B' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'C' => [
            0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110,
        ],
        'D' => [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'F' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'H' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'G' => [
            0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110,
        ],
        'I' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111,
        ],
        'J' => [
            0b00111, 0b00010, 0b00010, 0b00010, 0b10010, 0b10010, 0b01100,
        ],
        'K' => [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
        'L' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'M' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        'N' => [
            0b10001, 0b11001, 0b11001, 0b10101, 0b10011, 0b10011, 0b10001,
        ],
        'O' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'P' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'Q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        'S' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        'T' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'U' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'V' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ],
        'W' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010,
        ],
        'X' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ],
        'Y' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'Z' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        '!' => [
            0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00000, 0b00100,
        ],
        ',' => [
            0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00100, 0b01000,
        ],
        ' ' => [0; 7],
        _ => return None,
    };
    Some(glyph)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{OutlineFont, glyph};
    use layout::FontMetrics;

    /// Installed fonts to exercise [`OutlineFont`] with. Tests that need one
    /// return early on machines without any of them.
    pub(crate) fn system_font() -> Option<OutlineFont> {
        OutlineFont::load_first([
            "/System/Library/Fonts/Supplemental/Times New Roman.ttf",
            "/System/Library/Fonts/Helvetica.ttc",
            "C:/Windows/Fonts/times.ttf",
            "C:/Windows/Fonts/arial.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf",
        ])
    }

    #[test]
    fn outline_font_rejects_data_that_is_not_a_font() {
        assert!(OutlineFont::from_bytes(b"not a font".to_vec()).is_none());
        assert!(OutlineFont::load_first(["/nonexistent/font.ttf"]).is_none());
    }

    #[test]
    fn outline_font_metrics_scale_with_font_size() {
        let Some(font) = system_font() else {
            return;
        };
        let small = font.vertical_metrics(16.0);
        let large = font.vertical_metrics(32.0);
        assert!(small.ascent > 0.0 && small.descent > 0.0);
        assert!((large.ascent - small.ascent * 2.0).abs() < 1e-3);
        assert!((font.advance('W', 32.0) - font.advance('W', 16.0) * 2.0).abs() < 1e-3);
        // A proportional font: `W` is wider than `i`.
        assert!(font.advance('W', 16.0) > font.advance('i', 16.0));
        // Ascent plus descent is roughly one em for text fonts.
        let height = small.ascent + small.descent;
        assert!((12.0..24.0).contains(&height), "content height {height}");
    }

    #[test]
    fn supports_uppercase_and_lowercase_m() {
        assert_eq!(glyph('M'), glyph('m'));
        assert_eq!(
            glyph('M'),
            Some([
                0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001
            ])
        );
    }

    #[test]
    fn supports_every_ascii_letter() {
        for uppercase in b'A'..=b'Z' {
            let uppercase = char::from(uppercase);
            let lowercase = uppercase.to_ascii_lowercase();

            assert!(glyph(uppercase).is_some(), "missing glyph for {uppercase}");
            assert_eq!(glyph(lowercase), glyph(uppercase));
        }
    }
}
