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
mod tests {
    use super::glyph;

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
