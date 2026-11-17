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
        let size = data.len();
        let font = match ab_glyph::FontArc::try_from_vec(data) {
            Ok(font) => font,
            Err(error) => {
                log::debug!("font data ({size} bytes) could not be parsed: {error}");
                return None;
            }
        };
        let Some(units_per_em) = ab_glyph::Font::units_per_em(&font) else {
            log::debug!("font data ({size} bytes) has no units-per-em; it cannot be used");
            return None;
        };
        log::debug!("font loaded: {size} bytes, {units_per_em} units per em");
        Some(Self { font, units_per_em })
    }

    /// Parses a downloaded web font: WOFF2, WOFF, TrueType, OpenType, or a
    /// collection (first face), recognized by its leading signature.
    ///
    /// Returns `None` when the data cannot be decompressed or parsed.
    pub fn from_web_font_bytes(data: &[u8]) -> Option<Self> {
        let decoded = match data.get(..4) {
            Some(b"wOF2") => wuff::decompress_woff2(data),
            Some(b"wOFF") => wuff::decompress_woff1(data),
            _ => return Self::from_bytes(data.to_vec()),
        };
        match decoded {
            Ok(sfnt) => Self::from_bytes(sfnt),
            Err(error) => {
                log::debug!(
                    "web font ({} bytes) not decompressed: {error:?}",
                    data.len()
                );
                None
            }
        }
    }

    /// Loads the first of `paths` that reads and parses as a font, or `None`
    /// when none does.
    pub fn load_first<P: AsRef<std::path::Path>>(
        paths: impl IntoIterator<Item = P>,
    ) -> Option<Self> {
        let font = paths.into_iter().find_map(|path| {
            let path = path.as_ref();
            match std::fs::read(path) {
                Ok(data) => {
                    let font = Self::from_bytes(data);
                    if font.is_some() {
                        log::debug!("font loaded from {}", path.display());
                    }
                    font
                }
                Err(error) => {
                    log::trace!("font candidate {} not readable: {error}", path.display());
                    None
                }
            }
        });
        if font.is_none() {
            // Debug, not warn: the caller knows what the font was for and
            // reports the fallback once (e.g. the shell's page font).
            log::debug!(
                "no candidate font file could be loaded; text falls back to the built-in bitmap font"
            );
        }
        font
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

/// One downloadable face from an `@font-face` rule.
#[derive(Debug, Clone)]
struct WebFace {
    /// The rule's `font-family`, matched ignoring ASCII case.
    family: String,
    /// The inclusive `font-weight` range the face covers.
    weight: (u16, u16),
    italic: bool,
    font: OutlineFont,
}

/// The fonts one page's text is laid out and drawn with: a default font
/// plus the page's `@font-face` web fonts.
///
/// Layout measures through its [`layout::FontMetrics`] implementation, which
/// picks faces with [`layout::FontMetrics::select_font`], and
/// [`crate::Compositor::compose_page`] draws each run with the face layout
/// chose. [`layout::FontId::DEFAULT`] is the default font: an
/// [`OutlineFont`], or the built-in [`BitmapFont`] when there is none;
/// `FontId(n)` for `n > 0` is the `n`th added face.
///
/// Matching follows CSS Fonts 4 §5.2 in simplified form: families are tried
/// in order (names compared ignoring ASCII case; generic families such as
/// `serif` match only faces declared with that name), faces with the wanted
/// style are preferred, then the face whose weight range is nearest using
/// the §5.2 weight rules. Limitations: there are no system fonts besides the
/// default, no synthetic bold or italic, and no per-character fallback, so
/// characters missing from a web font draw its missing-glyph shape;
/// `unicode-range` is ignored.
#[derive(Debug, Clone, Default)]
pub struct FontCollection {
    default: Option<OutlineFont>,
    faces: Vec<WebFace>,
}

impl FontCollection {
    /// A collection with only a default font; `None` uses [`BitmapFont`].
    pub fn new(default: Option<OutlineFont>) -> Self {
        Self {
            default,
            faces: Vec::new(),
        }
    }

    /// Parses web font `data` (see [`OutlineFont::from_web_font_bytes`]) and
    /// adds it as a face of `family` covering the `weight` range (inclusive,
    /// 1–1000) in an upright or `italic` style. Returns `false`, adding
    /// nothing, when the data is not a usable font.
    pub fn add_face(
        &mut self,
        family: &str,
        weight: (u16, u16),
        italic: bool,
        data: &[u8],
    ) -> bool {
        let Some(font) = OutlineFont::from_web_font_bytes(data) else {
            return false;
        };
        self.add_font(family, weight, italic, font);
        true
    }

    /// Adds an already parsed font as a face; see [`Self::add_face`].
    pub fn add_font(&mut self, family: &str, weight: (u16, u16), italic: bool, font: OutlineFont) {
        log::debug!("font face added: {family:?} weight {weight:?} italic {italic}");
        self.faces.push(WebFace {
            family: family.to_owned(),
            weight: (weight.0.min(weight.1), weight.0.max(weight.1)),
            italic,
            font,
        });
    }

    /// Copies every face of `other` into this collection, after its own.
    /// Font data is shared, not parsed again.
    pub fn extend_faces(&mut self, other: &FontCollection) {
        self.faces.extend(other.faces.iter().cloned());
    }

    /// The default font, or `None` when text falls back to [`BitmapFont`].
    pub fn default_font(&self) -> Option<&OutlineFont> {
        self.default.as_ref()
    }

    /// How many web font faces were added.
    pub fn face_count(&self) -> usize {
        self.faces.len()
    }

    /// The outline font for `font`: the added face, or the default font for
    /// [`layout::FontId::DEFAULT`] and unknown ids. `None` means the bitmap
    /// font.
    pub fn outline(&self, font: layout::FontId) -> Option<&OutlineFont> {
        (font.0 as usize)
            .checked_sub(1)
            .and_then(|index| self.faces.get(index))
            .map(|face| &face.font)
            .or(self.default.as_ref())
    }

    fn metrics(&self, font: layout::FontId) -> &dyn layout::FontMetrics {
        match self.outline(font) {
            Some(outline) => outline,
            None => &BitmapFont,
        }
    }
}

/// How well a face's weight range suits a wanted weight, smaller is better
/// (CSS Fonts 4 §5.2, step 4 for `font-weight`).
fn weight_rank(wanted: u16, (low, high): (u16, u16)) -> (u8, u16) {
    let nearest = wanted.clamp(low, high);
    if nearest == wanted {
        return (0, 0);
    }
    let above = nearest > wanted;
    let distance = nearest.abs_diff(wanted);
    let preferred = match wanted {
        400..=500 => {
            return match (above, nearest <= 500) {
                (true, true) => (1, distance),
                (false, _) => (2, distance),
                (true, false) => (3, distance),
            };
        }
        0..400 => !above,
        _ => above,
    };
    (if preferred { 1 } else { 2 }, distance)
}

impl layout::FontMetrics for FontCollection {
    fn advance(&self, character: char, font_size: f32) -> f32 {
        self.metrics(layout::FontId::DEFAULT)
            .advance(character, font_size)
    }

    fn vertical_metrics(&self, font_size: f32) -> layout::VerticalMetrics {
        self.metrics(layout::FontId::DEFAULT)
            .vertical_metrics(font_size)
    }

    fn select_font(&self, families: &[String], weight: u16, italic: bool) -> layout::FontId {
        for family in families {
            let candidates = || {
                self.faces
                    .iter()
                    .enumerate()
                    .filter(|(_, face)| face.family.eq_ignore_ascii_case(family))
            };
            let styled = candidates().any(|(_, face)| face.italic == italic);
            let best = candidates()
                .filter(|(_, face)| !styled || face.italic == italic)
                .min_by_key(|(index, face)| (weight_rank(weight, face.weight), *index));
            if let Some((index, _)) = best {
                return layout::FontId(index as u32 + 1);
            }
        }
        layout::FontId::DEFAULT
    }

    fn font_advance(&self, font: layout::FontId, character: char, font_size: f32) -> f32 {
        self.metrics(font).advance(character, font_size)
    }

    fn font_vertical_metrics(
        &self,
        font: layout::FontId,
        font_size: f32,
    ) -> layout::VerticalMetrics {
        self.metrics(font).vertical_metrics(font_size)
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

    /// Wraps an sfnt (TrueType/OpenType) file in a WOFF 1.0 container with
    /// every table stored uncompressed, which the format allows.
    fn woff1(sfnt: &[u8]) -> Vec<u8> {
        let u16_at = |offset: usize| u16::from_be_bytes([sfnt[offset], sfnt[offset + 1]]);
        let u32_at =
            |offset: usize| u32::from_be_bytes(sfnt[offset..offset + 4].try_into().unwrap());
        let count = usize::from(u16_at(4));
        let tables: Vec<_> = (0..count)
            .map(|index| {
                let record = 12 + 16 * index;
                (
                    u32_at(record),
                    u32_at(record + 4),
                    u32_at(record + 8) as usize,
                    u32_at(record + 12) as usize,
                )
            })
            .collect();
        let padded = |length: usize| (length + 3) & !3;
        let sfnt_size = 12 + 16 * count + tables.iter().map(|t| padded(t.3)).sum::<usize>();
        let mut directory = Vec::new();
        let mut data = Vec::new();
        let data_start = 44 + 20 * count;
        for (tag, checksum, offset, length) in &tables {
            directory.extend(tag.to_be_bytes());
            directory.extend(((data_start + data.len()) as u32).to_be_bytes());
            directory.extend((*length as u32).to_be_bytes());
            directory.extend((*length as u32).to_be_bytes());
            directory.extend(checksum.to_be_bytes());
            data.extend(&sfnt[*offset..offset + length]);
            data.resize(padded(data.len()), 0);
        }
        let mut woff = b"wOFF".to_vec();
        woff.extend(u32_at(0).to_be_bytes());
        woff.extend(((data_start + data.len()) as u32).to_be_bytes());
        woff.extend((count as u16).to_be_bytes());
        woff.extend(0_u16.to_be_bytes());
        woff.extend((sfnt_size as u32).to_be_bytes());
        woff.extend([0, 1, 0, 0]);
        woff.extend([0; 20]);
        woff.extend(directory);
        woff.extend(data);
        woff
    }

    fn system_font_bytes() -> Option<Vec<u8>> {
        [
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf",
            "C:/Windows/Fonts/arial.ttf",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
        ]
        .into_iter()
        .find_map(|path| std::fs::read(path).ok())
    }

    #[test]
    fn web_fonts_load_from_woff_and_raw_sfnt_data() {
        let Some(sfnt) = system_font_bytes() else {
            return;
        };
        let raw = OutlineFont::from_web_font_bytes(&sfnt).expect("TrueType parses");
        let wrapped = OutlineFont::from_web_font_bytes(&woff1(&sfnt)).expect("WOFF parses");
        assert_eq!(wrapped.advance('W', 16.0), raw.advance('W', 16.0));
        assert!(OutlineFont::from_web_font_bytes(b"wOF2 truncated").is_none());
        assert!(OutlineFont::from_web_font_bytes(b"wOFF").is_none());
    }

    #[test]
    fn font_collection_matches_family_style_then_nearest_weight() {
        use super::{FontCollection, weight_rank};
        use layout::FontId;
        let Some(font) = system_font() else {
            return;
        };
        let mut fonts = FontCollection::new(None);
        fonts.add_font("Brand", (400, 400), false, font.clone()); // FontId(1)
        fonts.add_font("Brand", (700, 700), false, font.clone()); // FontId(2)
        fonts.add_font("brand", (300, 300), true, font.clone()); // FontId(3)
        fonts.add_font("Wide", (100, 900), false, font); // FontId(4)
        let families = |names: &[&str]| {
            names
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>()
        };
        let select =
            |names: &[&str], weight, italic| fonts.select_font(&families(names), weight, italic);
        assert_eq!(select(&["Brand"], 400, false), FontId(1));
        assert_eq!(select(&["BRAND"], 700, false), FontId(2));
        // 600 prefers heavier faces first.
        assert_eq!(select(&["Brand"], 600, false), FontId(2));
        // 450 prefers 450..=500, then lighter: 400 beats 700.
        assert_eq!(select(&["Brand"], 450, false), FontId(1));
        // Italic wins over weight when an italic face exists.
        assert_eq!(select(&["Brand"], 700, true), FontId(3));
        assert_eq!(select(&["Missing", "wide"], 550, false), FontId(4));
        assert_eq!(select(&["Missing", "serif"], 400, false), FontId::DEFAULT);
        // Without a default outline font, the default id measures with the
        // bitmap font, and faces with their own font.
        assert!(fonts.outline(FontId::DEFAULT).is_none());
        assert!(fonts.outline(FontId(2)).is_some());
        assert_eq!(fonts.font_advance(FontId::DEFAULT, 'W', 10.0), 6.0);
        assert_ne!(fonts.font_advance(FontId(1), 'i', 10.0), 6.0);

        assert_eq!(weight_rank(400, (300, 500)), (0, 0));
        assert!(weight_rank(300, (200, 200)) < weight_rank(300, (400, 400)));
        assert!(weight_rank(800, (900, 900)) < weight_rank(800, (700, 700)));
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
