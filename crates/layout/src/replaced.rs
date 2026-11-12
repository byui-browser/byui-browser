//! Replaced elements: `<img>` and inline `<svg>` (CSS 2.2 §10.3.2, §10.4,
//! §10.6.2, and CSS Images 3 §4.3 default sizing).
//!
//! Layout never decodes images. The embedder supplies each image's natural
//! size through [`ImageMetrics`], the same way [`crate::FontMetrics`] supplies
//! text measurements, and the renderer later draws the pixels for the
//! [`ImageKey`] stored on the box.

use html::NodeID;

use crate::Size;
use crate::style::{ComputedStyle, Dimension};

/// Identifies the picture a replaced box shows, so the renderer can look up
/// its pixels.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ImageKey {
    /// An `<img>` element's `src` attribute, trimmed but otherwise exactly as
    /// written in the document (not resolved against the base URL). The
    /// embedder fetches the resolved URL and files the result under this
    /// string.
    Src(String),
    /// An inline `<svg>` element in the document, identified by its node. The
    /// embedder serializes the element's subtree to SVG markup.
    InlineSvg(NodeID),
}

/// Reports the natural (intrinsic) size of images for layout.
///
/// Implemented by the embedder's decoded-image store (for example
/// `render::ImageStore`). Return `None` when the image failed to load or is
/// unknown; layout then treats it as a broken image.
pub trait ImageMetrics {
    /// The natural width and height of `key`'s image in CSS pixels, or `None`
    /// when no image is available.
    ///
    /// Layout only asks about [`ImageKey::Src`] keys: inline SVG is sized from
    /// the `<svg>` element's own `width`, `height`, and `viewBox` attributes.
    fn natural_size(&self, key: &ImageKey) -> Option<Size>;
}

/// An [`ImageMetrics`] with no images: every `<img>` is broken.
///
/// [`crate::layout_document`] uses this.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoImages;

impl ImageMetrics for NoImages {
    fn natural_size(&self, _key: &ImageKey) -> Option<Size> {
        None
    }
}

/// The natural dimensions of a replaced element's content (CSS Images 3
/// §4.1). Any of them may be missing; inline SVG with only a `viewBox`, for
/// example, has a ratio but no width or height.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct NaturalSize {
    pub width: Option<f32>,
    pub height: Option<f32>,
    /// Width divided by height.
    pub ratio: Option<f32>,
}

impl NaturalSize {
    /// A fully sized image, such as a decoded bitmap.
    pub(crate) fn from_size(size: Size) -> Self {
        Self {
            width: Some(size.width),
            height: Some(size.height),
            ratio: (size.width > 0.0 && size.height > 0.0).then(|| size.width / size.height),
        }
    }
}

/// The default object size for replaced content with no natural size
/// (CSS Images 3 §4.3, CSS 2.2 §10.3.2).
const DEFAULT_WIDTH: f32 = 300.0;
const DEFAULT_HEIGHT: f32 = 150.0;

/// A replaced element before layout.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReplacedBox {
    /// What the box paints.
    pub key: ImageKey,
    pub natural: NaturalSize,
    /// The element's `width` attribute, which applies when the `width`
    /// property is `auto` (the HTML presentational hint, HTML §15.4.3).
    pub width_hint: Dimension,
    /// The element's `height` attribute, as for `width_hint`.
    pub height_hint: Dimension,
}

impl ReplacedBox {
    /// The used content-box width and height (CSS 2.2 §10.3.2, §10.4,
    /// §10.6.2, §10.7).
    ///
    /// `cb_width` and `cb_height` are the containing block's size, `None` when
    /// indefinite (percentages then behave as `auto` or `none`). With both
    /// dimensions `auto` and a ratio but no natural size, the box fills the
    /// containing block's width (CSS 2.2 leaves this undefined; this follows
    /// browsers), or uses 300px when that is indefinite too.
    pub(crate) fn used_size(
        &self,
        style: &ComputedStyle,
        cb_width: Option<f32>,
        cb_height: Option<f32>,
    ) -> (f32, f32) {
        let pick = |property: Dimension, hint: Dimension| match property {
            Dimension::Auto => hint,
            other => other,
        };
        let width = pick(style.width, self.width_hint).resolve(cb_width);
        let height = pick(style.height, self.height_hint).resolve(cb_height);
        let min_width = style.min_width.resolve_or_zero(cb_width);
        let min_height = style.min_height.resolve_or_zero(cb_height);
        // `max-*: none` is stored as `Auto`, which resolves to `None`.
        let max_width = style
            .max_width
            .resolve(cb_width)
            .map(|max| max.max(min_width));
        let max_height = style
            .max_height
            .resolve(cb_height)
            .map(|max| max.max(min_height));
        let clamp = |value: f32, min: f32, max: Option<f32>| {
            max.map_or(value, |max| value.min(max)).max(min)
        };
        let natural = self.natural;

        if let (None, None, Some(ratio)) = (width, height, natural.ratio) {
            let (w, h) = match (natural.width, natural.height) {
                (Some(w), Some(h)) => (w, h),
                (Some(w), None) => (w, w / ratio),
                (None, Some(h)) => (h * ratio, h),
                (None, None) => {
                    let w = cb_width.unwrap_or(DEFAULT_WIDTH);
                    (w, w / ratio)
                }
            };
            return constrain_with_ratio((w, h), (min_width, max_width), (min_height, max_height));
        }

        let tentative_width = width.unwrap_or_else(|| match (height, natural.ratio) {
            (Some(h), Some(ratio)) => h * ratio,
            _ => natural.width.unwrap_or(DEFAULT_WIDTH),
        });
        let used_width = clamp(tentative_width, min_width, max_width);
        let tentative_height = height.unwrap_or_else(|| match natural.ratio {
            Some(ratio) => used_width / ratio,
            None => natural.height.unwrap_or(DEFAULT_HEIGHT),
        });
        (used_width, clamp(tentative_height, min_height, max_height))
    }
}

/// Resolves min/max constraints for replaced content with a ratio and both
/// dimensions `auto`, keeping the ratio where possible (the table in
/// CSS 2.2 §10.4).
fn constrain_with_ratio(
    (w, h): (f32, f32),
    (min_w, max_w): (f32, Option<f32>),
    (min_h, max_h): (f32, Option<f32>),
) -> (f32, f32) {
    let max_w = max_w.unwrap_or(f32::INFINITY);
    let max_h = max_h.unwrap_or(f32::INFINITY);
    if w <= 0.0 || h <= 0.0 {
        return (w.clamp(min_w, max_w), h.clamp(min_h, max_h));
    }
    let over_w = w > max_w;
    let under_w = w < min_w;
    let over_h = h > max_h;
    let under_h = h < min_h;
    match (over_w, under_w, over_h, under_h) {
        (false, false, false, false) => (w, h),
        (true, _, false, false) => (max_w, (max_w * h / w).max(min_h)),
        (_, true, false, false) => (min_w, (min_w * h / w).min(max_h)),
        (false, false, true, _) => ((max_h * w / h).max(min_w), max_h),
        (false, false, _, true) => ((min_h * w / h).min(max_w), min_h),
        (true, _, true, _) => {
            if max_w / w <= max_h / h {
                (max_w, (max_w * h / w).max(min_h))
            } else {
                ((max_h * w / h).max(min_w), max_h)
            }
        }
        (_, true, _, true) => {
            if min_w / w <= min_h / h {
                ((min_h * w / h).min(max_w), min_h)
            } else {
                (min_w, (min_w * h / w).min(max_h))
            }
        }
        (_, true, true, _) => (min_w, max_h),
        (true, _, _, true) => (max_w, min_h),
    }
}

/// Parses an `<svg>` or `<img>` `width`/`height` attribute: a non-negative
/// number of CSS pixels, a length with a unit, or a percentage.
pub(crate) fn parse_dimension_attribute(value: &str) -> Dimension {
    let value = value.trim();
    if let Some(percent) = css::values::parse_percentage(value) {
        return if percent >= 0.0 {
            Dimension::Percent(percent)
        } else {
            Dimension::Auto
        };
    }
    let number = value
        .parse::<f32>()
        .ok()
        .or_else(|| value.strip_suffix("px")?.trim().parse::<f32>().ok());
    match number {
        Some(px) if px.is_finite() && px >= 0.0 => Dimension::Px(px),
        _ => Dimension::Auto,
    }
}

/// The aspect ratio from an SVG `viewBox` attribute (`min-x min-y width
/// height`, separated by spaces and/or commas), or `None` when it is invalid.
pub(crate) fn view_box_ratio(value: &str) -> Option<f32> {
    let numbers = value
        .split(|character: char| character.is_ascii_whitespace() || character == ',')
        .filter(|part| !part.is_empty())
        .map(str::parse::<f32>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    match numbers[..] {
        [_, _, width, height] if width > 0.0 && height > 0.0 => Some(width / height),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replaced(natural: NaturalSize) -> ReplacedBox {
        ReplacedBox {
            key: ImageKey::Src("a.png".into()),
            natural,
            width_hint: Dimension::Auto,
            height_hint: Dimension::Auto,
        }
    }

    fn bitmap(width: f32, height: f32) -> NaturalSize {
        NaturalSize::from_size(Size { width, height })
    }

    #[test]
    fn auto_size_uses_the_natural_size() {
        let style = ComputedStyle::anonymous(None);
        assert_eq!(
            replaced(bitmap(40.0, 20.0)).used_size(&style, Some(500.0), None),
            (40.0, 20.0)
        );
    }

    #[test]
    fn one_specified_dimension_keeps_the_ratio() {
        let mut style = ComputedStyle::anonymous(None);
        style.width = Dimension::Px(80.0);
        assert_eq!(
            replaced(bitmap(40.0, 20.0)).used_size(&style, Some(500.0), None),
            (80.0, 40.0)
        );
        let mut style = ComputedStyle::anonymous(None);
        style.height = Dimension::Px(10.0);
        assert_eq!(
            replaced(bitmap(40.0, 20.0)).used_size(&style, Some(500.0), None),
            (20.0, 10.0)
        );
    }

    #[test]
    fn attribute_hints_apply_only_when_the_property_is_auto() {
        let mut image = replaced(bitmap(40.0, 20.0));
        image.width_hint = Dimension::Px(100.0);
        let style = ComputedStyle::anonymous(None);
        assert_eq!(image.used_size(&style, Some(500.0), None), (100.0, 50.0));
        let mut style = ComputedStyle::anonymous(None);
        style.width = Dimension::Px(60.0);
        assert_eq!(image.used_size(&style, Some(500.0), None), (60.0, 30.0));
    }

    #[test]
    fn max_width_percent_scales_down_keeping_the_ratio() {
        let mut style = ComputedStyle::anonymous(None);
        style.max_width = Dimension::Percent(100.0);
        assert_eq!(
            replaced(bitmap(1000.0, 500.0)).used_size(&style, Some(200.0), None),
            (200.0, 100.0)
        );
        // Without a definite containing block the percentage is ignored.
        assert_eq!(
            replaced(bitmap(1000.0, 500.0)).used_size(&style, None, None),
            (1000.0, 500.0)
        );
    }

    #[test]
    fn ratio_only_content_fills_the_containing_block() {
        let style = ComputedStyle::anonymous(None);
        let natural = NaturalSize {
            ratio: Some(2.0),
            ..NaturalSize::default()
        };
        assert_eq!(
            replaced(natural).used_size(&style, Some(300.0), None),
            (300.0, 150.0)
        );
    }

    #[test]
    fn content_without_any_natural_dimension_uses_300_by_150() {
        let style = ComputedStyle::anonymous(None);
        assert_eq!(
            replaced(NaturalSize::default()).used_size(&style, Some(800.0), None),
            (300.0, 150.0)
        );
    }

    #[test]
    fn min_max_table_handles_conflicting_constraints() {
        // Too wide and too short: max-width wins, then min-height.
        assert_eq!(
            constrain_with_ratio((400.0, 100.0), (0.0, Some(200.0)), (80.0, None)),
            (200.0, 80.0)
        );
        // Too tall only.
        assert_eq!(
            constrain_with_ratio((100.0, 400.0), (0.0, None), (0.0, Some(200.0))),
            (50.0, 200.0)
        );
    }

    #[test]
    fn attribute_and_view_box_parsing() {
        assert_eq!(parse_dimension_attribute("24"), Dimension::Px(24.0));
        assert_eq!(parse_dimension_attribute(" 24px "), Dimension::Px(24.0));
        assert_eq!(parse_dimension_attribute("50%"), Dimension::Percent(50.0));
        assert_eq!(parse_dimension_attribute("-3"), Dimension::Auto);
        assert_eq!(parse_dimension_attribute("wide"), Dimension::Auto);
        assert_eq!(view_box_ratio("0 0 24 12"), Some(2.0));
        assert_eq!(view_box_ratio("0,0,10,10"), Some(1.0));
        assert_eq!(view_box_ratio("0 0 0 10"), None);
        assert_eq!(view_box_ratio("0 0 10"), None);
    }
}
