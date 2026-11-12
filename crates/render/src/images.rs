//! Decoded images for replaced elements (`<img>` and inline `<svg>`).

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use layout::ImageKey;
use resvg::{tiny_skia, usvg};

/// The most device pixels a resampled copy may have. Pictures painted larger
/// than this are sampled directly instead of cached, so a huge zoomed image
/// cannot exhaust memory.
const MAX_CACHED_PIXELS: u64 = 16 * 1024 * 1024;

/// How many resampled copies [`ImageStore`] keeps before starting over.
const MAX_CACHED_COPIES: usize = 64;

/// RGBA8 pixels with straight (non-premultiplied) alpha, row-major.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Bitmap {
    /// The straight-alpha pixel at `(x, y)`; the caller keeps it in bounds.
    pub(crate) fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let offset = ((y * self.width + x) * 4) as usize;
        let mut pixel = [0; 4];
        pixel.copy_from_slice(&self.rgba[offset..offset + 4]);
        pixel
    }

    /// This bitmap resampled to `width` × `height`, filtering in
    /// premultiplied alpha so transparent pixels do not bleed their color.
    fn resized(&self, width: u32, height: u32) -> Self {
        if (width, height) == (self.width, self.height) {
            return self.clone();
        }
        let mut premultiplied = self.rgba.clone();
        for pixel in premultiplied.chunks_exact_mut(4) {
            let alpha = u16::from(pixel[3]);
            for channel in &mut pixel[..3] {
                *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
            }
        }
        let source = image::RgbaImage::from_raw(self.width, self.height, premultiplied)
            .expect("bitmap buffers hold width * height pixels");
        let mut resized = image::imageops::resize(
            &source,
            width,
            height,
            image::imageops::FilterType::Triangle,
        )
        .into_raw();
        unpremultiply(&mut resized);
        Self {
            width,
            height,
            rgba: resized,
        }
    }
}

impl fmt::Debug for Bitmap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Bitmap({}x{})", self.width, self.height)
    }
}

/// Converts premultiplied RGBA8 pixels to straight alpha in place.
fn unpremultiply(rgba: &mut [u8]) {
    for pixel in rgba.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        if alpha == 0 {
            pixel[..3].fill(0);
        } else if alpha < 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u16::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
}

/// One decoded picture.
enum Picture {
    /// A raster image, such as a decoded PNG or JPEG.
    Bitmap(Bitmap),
    /// A parsed SVG document, rasterized at the size it is painted.
    Svg(Box<usvg::Tree>),
}

/// Decoded pictures for one document, keyed like layout's [`ImageKey`].
///
/// The embedder fills the store from fetched files with
/// [`Self::insert_encoded`] and from inline `<svg>` markup with
/// [`Self::insert_svg`], passes it to `layout::layout_document_with_images`
/// (through its [`layout::ImageMetrics`] implementation) to size replaced
/// boxes, and to [`crate::Compositor::compose_page`] to draw them.
///
/// Supported formats: PNG, JPEG, GIF (first frame only), WebP, BMP, ICO, and
/// SVG (plain or gzip-compressed). SVG `<text>` and raster `<image>` elements
/// inside SVG are not rendered, and SVG is drawn without access to the page's
/// CSS. Bitmaps are resampled with a triangle filter to the size they are
/// painted at; resampled copies and rasterized SVGs are cached per painted
/// size, so repainting a scrolled page does not redo the work.
#[derive(Default)]
pub struct ImageStore {
    pictures: HashMap<ImageKey, Arc<Picture>>,
    /// Pictures already resampled or rasterized to a device-pixel size.
    scaled: Mutex<HashMap<(ImageKey, u32, u32), Arc<Bitmap>>>,
}

impl ImageStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Decodes an image file and stores it under `key`, replacing any picture
    /// already stored there.
    ///
    /// SVG is recognized by its content (markup, or gzip-compressed SVGZ);
    /// everything else is decoded by guessing the bitmap format from its
    /// leading bytes. Returns `false`, storing nothing, when `bytes` is not a
    /// supported image.
    pub fn insert_encoded(&mut self, key: ImageKey, bytes: &[u8]) -> bool {
        let picture = if looks_like_svg(bytes) {
            parse_svg(bytes).map(|tree| Picture::Svg(Box::new(tree)))
        } else {
            match image::load_from_memory(bytes) {
                Ok(decoded) => {
                    let rgba = decoded.into_rgba8();
                    Some(Picture::Bitmap(Bitmap {
                        width: rgba.width(),
                        height: rgba.height(),
                        rgba: rgba.into_raw(),
                    }))
                }
                Err(error) => {
                    log::debug!("image {key:?} ({} bytes) not decoded: {error}", bytes.len());
                    None
                }
            }
        };
        self.store(key, picture)
    }

    /// Parses SVG `markup` produced by serializing an HTML document's inline
    /// `<svg>` element, and stores it under `key`.
    ///
    /// The HTML parser lowercases names and the HTML serializer writes
    /// `&nbsp;`, neither of which an XML parser accepts, so this first
    /// restores SVG's mixed-case element and attribute names (the tables in
    /// HTML §13.2.6.5), declares the SVG and XLink namespaces when the root
    /// element does not, and replaces `&nbsp;`. Returns `false`, storing
    /// nothing, when the markup still does not parse.
    pub fn insert_svg(&mut self, key: ImageKey, markup: &str) -> bool {
        let markup = xml_from_html_svg(markup);
        let picture = parse_svg(markup.as_bytes()).map(|tree| Picture::Svg(Box::new(tree)));
        self.store(key, picture)
    }

    fn store(&mut self, key: ImageKey, picture: Option<Picture>) -> bool {
        let Some(picture) = picture else {
            return false;
        };
        if let Ok(mut scaled) = self.scaled.lock() {
            scaled.retain(|(cached, _, _), _| *cached != key);
        }
        self.pictures.insert(key, Arc::new(picture));
        true
    }

    /// Copies every picture of `other` into this store, replacing pictures
    /// with the same keys. Pictures are shared, not decoded again.
    pub fn extend_from(&mut self, other: &ImageStore) {
        for (key, picture) in &other.pictures {
            self.pictures.insert(key.clone(), Arc::clone(picture));
        }
    }

    /// Whether a picture is stored under `key`.
    pub fn contains(&self, key: &ImageKey) -> bool {
        self.pictures.contains_key(key)
    }

    /// How many pictures are stored.
    pub fn len(&self) -> usize {
        self.pictures.len()
    }

    /// Whether no pictures are stored.
    pub fn is_empty(&self) -> bool {
        self.pictures.is_empty()
    }

    /// The natural size of `key`'s picture in CSS pixels: a bitmap's pixel
    /// size, or an SVG document's size from its root `width`, `height`, and
    /// `viewBox` (100×100 when it gives none).
    pub fn natural_size(&self, key: &ImageKey) -> Option<(f32, f32)> {
        Some(match self.pictures.get(key)?.as_ref() {
            Picture::Bitmap(bitmap) => (bitmap.width as f32, bitmap.height as f32),
            Picture::Svg(tree) => (tree.size().width(), tree.size().height()),
        })
    }

    /// `key`'s picture as a straight-alpha bitmap exactly `width` × `height`
    /// device pixels, from the cache when possible. `None` when there is no
    /// picture, the size is empty, or the copy would be too large to cache.
    pub(crate) fn scaled(&self, key: &ImageKey, width: u32, height: u32) -> Option<Arc<Bitmap>> {
        if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_CACHED_PIXELS {
            return None;
        }
        let picture = self.pictures.get(key)?;
        let cache_key = (key.clone(), width, height);
        if let Some(bitmap) = self
            .scaled
            .lock()
            .ok()
            .and_then(|cache| cache.get(&cache_key).cloned())
        {
            return Some(bitmap);
        }
        let bitmap = Arc::new(match picture.as_ref() {
            Picture::Bitmap(bitmap) => bitmap.resized(width, height),
            Picture::Svg(tree) => rasterize_svg(tree, width, height)?,
        });
        log::trace!("image {key:?} scaled to {width}x{height}");
        if let Ok(mut cache) = self.scaled.lock() {
            if cache.len() >= MAX_CACHED_COPIES {
                cache.clear();
            }
            cache.insert(cache_key, Arc::clone(&bitmap));
        }
        Some(bitmap)
    }

    /// `key`'s bitmap at its natural size, for drawing pictures too large to
    /// resample into the cache. SVG pictures have none.
    pub(crate) fn original(&self, key: &ImageKey) -> Option<&Bitmap> {
        match self.pictures.get(key)?.as_ref() {
            Picture::Bitmap(bitmap) => Some(bitmap),
            Picture::Svg(_) => None,
        }
    }
}

impl Clone for ImageStore {
    /// Shares the decoded pictures; the clone starts with an empty cache of
    /// resampled copies.
    fn clone(&self) -> Self {
        Self {
            pictures: self.pictures.clone(),
            scaled: Mutex::default(),
        }
    }
}

impl fmt::Debug for ImageStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageStore")
            .field("pictures", &self.pictures.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl layout::ImageMetrics for ImageStore {
    fn natural_size(&self, key: &ImageKey) -> Option<layout::Size> {
        let (width, height) = ImageStore::natural_size(self, key)?;
        Some(layout::Size { width, height })
    }
}

/// Whether `bytes` look like an SVG document (text starting with markup) or
/// gzip data, which is tried as compressed SVG.
fn looks_like_svg(bytes: &[u8]) -> bool {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    if bytes.starts_with(&[0x1f, 0x8b]) {
        return true;
    }
    bytes
        .iter()
        .find(|byte| !byte.is_ascii_whitespace())
        .is_some_and(|byte| *byte == b'<')
}

fn parse_svg(bytes: &[u8]) -> Option<usvg::Tree> {
    match usvg::Tree::from_data(bytes, &usvg::Options::default()) {
        Ok(tree) => Some(tree),
        Err(error) => {
            log::debug!("SVG ({} bytes) not parsed: {error}", bytes.len());
            None
        }
    }
}

/// Rasterizes `tree` into `width` × `height` pixels, scaled uniformly to
/// fit and centered, like SVG's default `preserveAspectRatio="xMidYMid
/// meet"` when the box's ratio differs from the document's.
fn rasterize_svg(tree: &usvg::Tree, width: u32, height: u32) -> Option<Bitmap> {
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
    let size = tree.size();
    let scale = (width as f32 / size.width()).min(height as f32 / size.height());
    let transform = tiny_skia::Transform::from_row(
        scale,
        0.0,
        0.0,
        scale,
        (width as f32 - size.width() * scale) / 2.0,
        (height as f32 - size.height() * scale) / 2.0,
    );
    resvg::render(tree, transform, &mut pixmap.as_mut());
    let mut rgba = pixmap.take();
    unpremultiply(&mut rgba);
    Some(Bitmap {
        width,
        height,
        rgba,
    })
}

/// SVG element names the HTML parser lowercases, in their XML spelling
/// (HTML §13.2.6.5, "adjust SVG tag name").
const SVG_ELEMENT_NAMES: &[&str] = &[
    "altGlyph",
    "altGlyphDef",
    "altGlyphItem",
    "animateColor",
    "animateMotion",
    "animateTransform",
    "clipPath",
    "feBlend",
    "feColorMatrix",
    "feComponentTransfer",
    "feComposite",
    "feConvolveMatrix",
    "feDiffuseLighting",
    "feDisplacementMap",
    "feDistantLight",
    "feDropShadow",
    "feFlood",
    "feFuncA",
    "feFuncB",
    "feFuncG",
    "feFuncR",
    "feGaussianBlur",
    "feImage",
    "feMerge",
    "feMergeNode",
    "feMorphology",
    "feOffset",
    "fePointLight",
    "feSpecularLighting",
    "feSpotLight",
    "feTile",
    "feTurbulence",
    "foreignObject",
    "glyphRef",
    "linearGradient",
    "radialGradient",
    "textPath",
];

/// SVG attribute names the HTML parser lowercases, in their XML spelling
/// (HTML §13.2.6.5, "adjust SVG attributes").
const SVG_ATTRIBUTE_NAMES: &[&str] = &[
    "attributeName",
    "attributeType",
    "baseFrequency",
    "baseProfile",
    "calcMode",
    "clipPathUnits",
    "diffuseConstant",
    "edgeMode",
    "filterUnits",
    "glyphRef",
    "gradientTransform",
    "gradientUnits",
    "kernelMatrix",
    "kernelUnitLength",
    "keyPoints",
    "keySplines",
    "keyTimes",
    "lengthAdjust",
    "limitingConeAngle",
    "markerHeight",
    "markerUnits",
    "markerWidth",
    "maskContentUnits",
    "maskUnits",
    "numOctaves",
    "pathLength",
    "patternContentUnits",
    "patternTransform",
    "patternUnits",
    "pointsAtX",
    "pointsAtY",
    "pointsAtZ",
    "preserveAlpha",
    "preserveAspectRatio",
    "primitiveUnits",
    "refX",
    "refY",
    "repeatCount",
    "repeatDur",
    "requiredExtensions",
    "requiredFeatures",
    "specularConstant",
    "specularExponent",
    "spreadMethod",
    "startOffset",
    "stdDeviation",
    "stitchTiles",
    "surfaceScale",
    "systemLanguage",
    "tableValues",
    "targetX",
    "targetY",
    "textLength",
    "viewBox",
    "viewTarget",
    "xChannelSelector",
    "yChannelSelector",
    "zoomAndPan",
];

/// Turns HTML-serialized inline SVG into XML an SVG parser accepts; see
/// [`ImageStore::insert_svg`].
fn xml_from_html_svg(markup: &str) -> String {
    let mut output = String::with_capacity(markup.len() + 96);
    let mut rest = markup;
    let mut first_tag = true;
    while let Some(open) = rest.find('<') {
        output.push_str(&rest[..open].replace("&nbsp;", "&#160;"));
        rest = &rest[open..];
        let Some(close) = tag_end(rest) else {
            break;
        };
        let tag = &rest[1..close];
        rest = &rest[close + 1..];
        if tag.starts_with('!') || tag.starts_with('?') {
            output.push('<');
            output.push_str(tag);
            output.push('>');
            continue;
        }
        let (slash, body) = match tag.strip_prefix('/') {
            Some(body) => ("/", body),
            None => ("", tag),
        };
        let name_end = body
            .find(|character: char| character.is_ascii_whitespace() || character == '/')
            .unwrap_or(body.len());
        let (name, attributes) = body.split_at(name_end);
        output.push('<');
        output.push_str(slash);
        output.push_str(restore_case(name, SVG_ELEMENT_NAMES));
        output.push_str(&restore_attribute_names(attributes).replace("&nbsp;", "&#160;"));
        if first_tag && slash.is_empty() {
            first_tag = false;
            output.push_str(&missing_root_size(attributes));
            if !attributes.contains("xmlns=") {
                output.push_str(" xmlns=\"http://www.w3.org/2000/svg\"");
            }
            if !attributes.contains("xmlns:xlink=") {
                output.push_str(" xmlns:xlink=\"http://www.w3.org/1999/xlink\"");
            }
        }
        output.push('>');
    }
    output.push_str(&rest.replace("&nbsp;", "&#160;"));
    output
}

/// ` width="…"` or ` height="…"` for a root `<svg>` that has a `viewBox`
/// and only one of the two, derived from the `viewBox` ratio. Without this an
/// SVG parser would default the missing one to the `viewBox`'s, changing
/// the document's ratio; HTML layout keeps the ratio instead.
fn missing_root_size(attributes: &str) -> String {
    let value = |name: &str| attribute_value(attributes, name);
    let number = |name: &str| {
        let text = value(name)?;
        let text = text.trim();
        text.strip_suffix("px")
            .unwrap_or(text)
            .trim()
            .parse::<f32>()
            .ok()
    };
    let Some(ratio) = value("viewbox").and_then(|view_box| {
        let numbers = view_box
            .split(|character: char| character.is_ascii_whitespace() || character == ',')
            .filter(|part| !part.is_empty())
            .filter_map(|part| part.parse::<f32>().ok())
            .collect::<Vec<_>>();
        match numbers[..] {
            [_, _, width, height] if width > 0.0 && height > 0.0 => Some(width / height),
            _ => None,
        }
    }) else {
        return String::new();
    };
    match (
        number("width"),
        value("height"),
        value("width"),
        number("height"),
    ) {
        (Some(width), None, _, _) => format!(" height=\"{}\"", width / ratio),
        (_, _, None, Some(height)) => format!(" width=\"{}\"", height * ratio),
        _ => String::new(),
    }
}

/// The value of attribute `name` (matched ignoring ASCII case) in a start
/// tag's attribute text.
fn attribute_value<'a>(attributes: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = attributes;
    loop {
        rest = rest.trim_start_matches(|character: char| {
            character.is_ascii_whitespace() || character == '/'
        });
        if rest.is_empty() {
            return None;
        }
        let end = rest
            .find(|character: char| character.is_ascii_whitespace() || character == '=')
            .unwrap_or(rest.len());
        let (found, after) = rest.split_at(end);
        let after = after.trim_start();
        let (value, remaining) = match after.strip_prefix('=') {
            Some(after) => {
                let after = after.trim_start();
                match after.chars().next() {
                    Some(quote @ ('"' | '\'')) => {
                        let body = &after[1..];
                        let close = body.find(quote).unwrap_or(body.len());
                        (&body[..close], body.get(close + 1..).unwrap_or(""))
                    }
                    _ => {
                        let close = after
                            .find(|character: char| character.is_ascii_whitespace())
                            .unwrap_or(after.len());
                        (&after[..close], &after[close..])
                    }
                }
            }
            None => ("", after),
        };
        if found.eq_ignore_ascii_case(name) {
            return Some(value);
        }
        rest = remaining;
    }
}

/// The index of the `>` closing the tag that starts `text`, skipping `>`
/// inside quoted attribute values.
fn tag_end(text: &str) -> Option<usize> {
    let mut quote = None;
    for (index, character) in text.char_indices().skip(1) {
        match (quote, character) {
            (Some(open), _) if character == open => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(character),
            (None, '>') => return Some(index),
            _ => {}
        }
    }
    None
}

/// `name` in the spelling `table` lists for it, or `name` unchanged.
fn restore_case<'a>(name: &'a str, table: &[&'a str]) -> &'a str {
    table
        .iter()
        .find(|known| known.eq_ignore_ascii_case(name))
        .copied()
        .unwrap_or(name)
}

/// Restores the case of attribute names in a start tag's attribute text,
/// leaving quoted values untouched.
fn restore_attribute_names(attributes: &str) -> String {
    let mut output = String::with_capacity(attributes.len());
    let mut quote = None;
    let mut name = String::new();
    for character in attributes.chars() {
        if let Some(open) = quote {
            output.push(character);
            if character == open {
                quote = None;
            }
            continue;
        }
        if character.is_ascii_alphanumeric() || matches!(character, '-' | ':' | '_') {
            name.push(character);
            continue;
        }
        if !name.is_empty() {
            output.push_str(restore_case(&name, SVG_ATTRIBUTE_NAMES));
            name.clear();
        }
        if matches!(character, '"' | '\'') {
            quote = Some(character);
        }
        output.push(character);
    }
    output.push_str(restore_case(&name, SVG_ATTRIBUTE_NAMES));
    output
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A PNG file of `width` × `height` pixels, all `rgba`.
    pub(crate) fn png(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba(rgba));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut bytes, image::ImageFormat::Png)
            .expect("PNG encodes");
        bytes.into_inner()
    }

    fn key(name: &str) -> ImageKey {
        ImageKey::Src(name.into())
    }

    #[test]
    fn decodes_png_and_reports_its_natural_size() {
        let mut store = ImageStore::new();
        assert!(store.insert_encoded(key("a.png"), &png(3, 2, [255, 0, 0, 255])));
        assert_eq!(store.natural_size(&key("a.png")), Some((3.0, 2.0)));
        let scaled = store.scaled(&key("a.png"), 6, 4).expect("scaled copy");
        assert_eq!((scaled.width, scaled.height), (6, 4));
        assert_eq!(scaled.pixel(5, 3), [255, 0, 0, 255]);
    }

    #[test]
    fn rejects_data_that_is_not_an_image() {
        let mut store = ImageStore::new();
        assert!(!store.insert_encoded(key("a"), b"GIF? no"));
        assert!(!store.insert_encoded(key("b"), b"<html>not svg</html>"));
        assert!(store.is_empty());
        assert_eq!(store.natural_size(&key("a")), None);
    }

    #[test]
    fn svg_files_rasterize_at_the_requested_size() {
        let mut store = ImageStore::new();
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="5">
            <rect width="5" height="5" fill="#00f"/></svg>"##;
        assert!(store.insert_encoded(key("a.svg"), svg));
        assert_eq!(store.natural_size(&key("a.svg")), Some((10.0, 5.0)));
        let bitmap = store.scaled(&key("a.svg"), 20, 10).expect("rasterized");
        assert_eq!(bitmap.pixel(2, 5), [0, 0, 255, 255]);
        assert_eq!(bitmap.pixel(15, 5)[3], 0);
    }

    #[test]
    fn inline_svg_markup_from_html_is_repaired_before_parsing() {
        let markup = "<svg viewbox=\"0 0 2 1\" width=\"20\"><lineargradient id=\"g\" \
                      gradientunits=\"userSpaceOnUse\"><stop offset=\"0\" stop-color=\"red\">\
                      </stop></lineargradient><rect width=\"1\" height=\"1\" fill=\"url(#g)\">\
                      </rect><text>a&nbsp;b</text></svg>";
        let xml = xml_from_html_svg(markup);
        assert!(xml.starts_with(
            "<svg viewBox=\"0 0 2 1\" width=\"20\" height=\"10\" \
             xmlns=\"http://www.w3.org/2000/svg\""
        ));
        assert!(xml.contains("<linearGradient id=\"g\" gradientUnits=\"userSpaceOnUse\">"));
        assert!(xml.contains("</linearGradient>"));
        assert!(xml.contains("a&#160;b"));
        let mut store = ImageStore::new();
        assert!(store.insert_svg(ImageKey::InlineSvg(html::NodeID::new(4)), markup));
        assert_eq!(
            store.natural_size(&ImageKey::InlineSvg(html::NodeID::new(4))),
            Some((20.0, 10.0))
        );
    }

    #[test]
    fn svg_in_a_box_of_another_ratio_is_letterboxed_not_stretched() {
        let mut store = ImageStore::new();
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1">
            <rect width="1" height="1" fill="#0f0"/></svg>"##;
        assert!(store.insert_encoded(key("square.svg"), svg));
        let bitmap = store
            .scaled(&key("square.svg"), 20, 10)
            .expect("rasterized");
        assert_eq!(bitmap.pixel(10, 5), [0, 255, 0, 255]);
        assert_eq!(bitmap.pixel(1, 5)[3], 0);
        assert_eq!(bitmap.pixel(18, 5)[3], 0);
    }

    #[test]
    fn root_size_is_completed_from_the_view_box() {
        assert_eq!(
            missing_root_size(" viewbox='0 0 4 2' width=\"8px\""),
            " height=\"4\""
        );
        assert_eq!(
            missing_root_size(" height=3 viewBox=\"0,0,4,2\""),
            " width=\"6\""
        );
        assert_eq!(missing_root_size(" viewbox='0 0 4 2'"), "");
        assert_eq!(missing_root_size(" width=1 height=1 viewbox='0 0 4 2'"), "");
    }

    #[test]
    fn attribute_values_keep_their_case() {
        assert_eq!(
            restore_attribute_names(" viewbox=\"Viewbox>\" class='refx'"),
            " viewBox=\"Viewbox>\" class='refx'"
        );
    }

    #[test]
    fn resampling_does_not_bleed_transparent_color() {
        // Transparent red next to opaque blue: the blend must stay blue.
        let bitmap = Bitmap {
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 0, 0, 0, 255, 255],
        };
        let resized = bitmap.resized(4, 1);
        for x in 0..4 {
            let [r, _, _, a] = resized.pixel(x, 0);
            assert!(a == 0 || r == 0, "pixel {x} is {:?}", resized.pixel(x, 0));
        }
    }

    #[test]
    fn huge_sizes_are_not_cached() {
        let mut store = ImageStore::new();
        store.insert_encoded(key("a.png"), &png(1, 1, [0, 0, 0, 255]));
        assert!(store.scaled(&key("a.png"), 5000, 5000).is_none());
        assert!(store.original(&key("a.png")).is_some());
    }
}
