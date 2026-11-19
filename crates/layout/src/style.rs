//! Typed computed values for the properties layout consumes.
//!
//! [`css::style_document`] produces cascaded, inherited declarations as text.
//! This module parses `display`, box dimensions and edges, `box-sizing`, text
//! colors, `text-align`, `line-height`, `border-*-radius`, `box-shadow`, `position`, `float`,
//! the inset properties (`top`, `right`, `bottom`, `left`), and the flex subset
//! (`flex-direction`, `flex-grow`, `flex-shrink`, `flex-basis`, and `gap`). Other
//! properties use their initial values. A missing or unparsable value falls back to the initial
//! value. Lengths are resolved to CSS pixels here; percentages are kept
//! because they resolve against the containing block during layout.

use std::rc::Rc;
use std::sync::Arc;

use css::values::{LengthContext, parse_length, parse_percentage};
use css::{CssColor, Declaration};

use crate::shadow::parse_box_shadow;
use crate::{BorderRadius, BoxShadow, CornerRadius, Corners, LengthPercentage, Sides};

/// The outer display type: how a box participates in its parent's formatting
/// context (CSS Display 3 §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DisplayOutside {
    Block,
    Inline,
}

/// The inner display type: the formatting context a box establishes for its
/// children (CSS Display 3 §2.2).
///
/// Inner formatting context established by a generated box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DisplayInside {
    Flow,
    FlowRoot,
    Flex,
    Grid,
    Table,
}

/// The computed `display` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Display {
    /// The element and its descendants generate no boxes.
    None,
    /// The element generates no box; its children are boxed as if they were
    /// children of its parent.
    Contents,
    /// The element generates a principal box.
    Box {
        outside: DisplayOutside,
        inside: DisplayInside,
    },
}

/// Whether `value` is a valid CSS-wide or color keyword that
/// [`css::parse_color`] maps to "no color", so it is not worth logging.
fn is_no_color_keyword(value: &str) -> bool {
    ["transparent", "inherit", "initial", "unset", "currentcolor"]
        .iter()
        .any(|keyword| value.eq_ignore_ascii_case(keyword))
}

/// Whether `value` is a CSS-wide keyword, which resolves to the initial value
/// for non-inherited properties.
fn is_css_wide_keyword(value: &str) -> bool {
    ["inherit", "initial", "unset", "revert", "revert-layer"]
        .iter()
        .any(|keyword| value.eq_ignore_ascii_case(keyword))
}

/// Parses a `border-*-radius` longhand: one or two non-negative
/// `<length-percentage>` values, horizontal then vertical. One value sets both.
fn parse_corner_radius(
    value: &str,
    context: &LengthContext,
) -> Option<CornerRadius<LengthPercentage>> {
    let component = |part: &str| {
        let parsed = match parse_percentage(part) {
            Some(percent) => LengthPercentage::Percent(percent),
            None => LengthPercentage::Px(parse_length(part)?.to_px(context)),
        };
        let (LengthPercentage::Px(amount) | LengthPercentage::Percent(amount)) = parsed;
        (amount.is_finite() && amount >= 0.0).then_some(parsed)
    };
    match *value.split_ascii_whitespace().collect::<Vec<_>>() {
        [both] => {
            let radius = component(both)?;
            Some(CornerRadius {
                horizontal: radius,
                vertical: radius,
            })
        }
        [horizontal, vertical] => Some(CornerRadius {
            horizontal: component(horizontal)?,
            vertical: component(vertical)?,
        }),
        _ => None,
    }
}

impl Display {
    const INLINE: Self = Self::Box {
        outside: DisplayOutside::Inline,
        inside: DisplayInside::Flow,
    };

    /// Parses `display`, including the multi-keyword syntax.
    ///
    /// `list-item` is accepted as a block box; list markers are not generated
    /// yet. Internal table values are laid out as independent blocks and ruby
    /// values as inlines.
    fn parse(value: &str) -> Option<Self> {
        let value = value.trim().to_ascii_lowercase();
        let block = |inside| Self::Box {
            outside: DisplayOutside::Block,
            inside,
        };
        let inline = |inside| Self::Box {
            outside: DisplayOutside::Inline,
            inside,
        };
        let single = match value.as_str() {
            "none" => Some(Self::None),
            "contents" => Some(Self::Contents),
            "block" | "list-item" => Some(block(DisplayInside::Flow)),
            "inline"
            | "ruby"
            | "ruby-base"
            | "ruby-text"
            | "ruby-base-container"
            | "ruby-text-container" => Some(inline(DisplayInside::Flow)),
            "inline-block" | "inline-table" | "inline-grid" => {
                Some(inline(DisplayInside::FlowRoot))
            }
            "inline-flex" => Some(inline(DisplayInside::Flex)),
            "flex" => Some(block(DisplayInside::Flex)),
            "grid" => Some(block(DisplayInside::Grid)),
            "table" => Some(block(DisplayInside::Table)),
            "flow-root" | "table-row-group" | "table-header-group" | "table-footer-group"
            | "table-row" | "table-cell" | "table-column-group" | "table-column"
            | "table-caption" => Some(block(DisplayInside::FlowRoot)),
            _ => None,
        };
        if single.is_some() {
            if matches!(
                value.as_str(),
                "table" | "inline-table" | "flex" | "inline-flex" | "grid" | "inline-grid"
            ) {
                log::trace!("display: {value} is not implemented; laying out as flow-root");
            }
            return single;
        }
        let mut outside = None;
        let mut inside = None;
        for keyword in value.split_ascii_whitespace() {
            match keyword {
                "block" if outside.is_none() => outside = Some(DisplayOutside::Block),
                "inline" if outside.is_none() => outside = Some(DisplayOutside::Inline),
                "flow" if inside.is_none() => inside = Some(DisplayInside::Flow),
                "flow-root" if inside.is_none() => inside = Some(DisplayInside::FlowRoot),
                "flex" if inside.is_none() => inside = Some(DisplayInside::Flex),
                "grid" if inside.is_none() => inside = Some(DisplayInside::Grid),
                "table" if inside.is_none() => inside = Some(DisplayInside::Table),
                "list-item" => {}
                _ => return None,
            }
        }
        Some(Self::Box {
            outside: outside.unwrap_or(DisplayOutside::Block),
            inside: inside.unwrap_or(DisplayInside::Flow),
        })
    }

    /// Blockifies the display type (CSS Display 3 §2.7), as required for the
    /// root element.
    pub(crate) fn blockified(self) -> Self {
        match self {
            Self::None => Self::None,
            Self::Contents => Self::Box {
                outside: DisplayOutside::Block,
                inside: DisplayInside::Flow,
            },
            Self::Box { inside, .. } => Self::Box {
                outside: DisplayOutside::Block,
                inside,
            },
        }
    }
}

/// The computed `position` value (CSS Positioned Layout 3 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Position {
    /// Normal flow; the inset properties do not apply.
    #[default]
    Static,
    /// Laid out in flow, then shifted by its insets.
    Relative,
    /// Out of flow, placed against the nearest positioned ancestor.
    Absolute,
    /// Out of flow, placed against the viewport.
    Fixed,
    /// Laid out in flow. Scroll-dependent sticking is not implemented, so it
    /// behaves as `relative` with zero offsets.
    Sticky,
}

impl Position {
    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "static" => Some(Self::Static),
            "relative" => Some(Self::Relative),
            "absolute" => Some(Self::Absolute),
            "fixed" => Some(Self::Fixed),
            "sticky" | "-webkit-sticky" => Some(Self::Sticky),
            _ => None,
        }
    }
}

/// The computed `float` value (CSS 2.2 §9.5.1). Text is always
/// left-to-right, so `inline-start` is `left` and `inline-end` is `right`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Float {
    #[default]
    None,
    Left,
    Right,
}

impl Float {
    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "none" => Some(Self::None),
            "left" | "inline-start" => Some(Self::Left),
            "right" | "inline-end" => Some(Self::Right),
            _ => None,
        }
    }
}

/// A sizing value with lengths resolved to CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Dimension {
    Auto,
    Px(f32),
    Percent(f32),
    MinContent,
    MaxContent,
    FitContent,
}

impl Dimension {
    /// Resolves against `basis`, returning `None` for `auto` or for a
    /// percentage whose basis is indefinite.
    pub(crate) fn resolve(self, basis: Option<f32>) -> Option<f32> {
        match self {
            Self::Auto => None,
            Self::Px(px) => Some(px),
            Self::Percent(percent) => basis.map(|basis| basis * percent / 100.0),
            Self::MinContent | Self::MaxContent | Self::FitContent => None,
        }
    }

    /// Resolves treating `auto` and indefinite percentages as zero.
    pub(crate) fn resolve_or_zero(self, basis: Option<f32>) -> f32 {
        self.resolve(basis).unwrap_or(0.0)
    }
}

/// The `text-transform` property: how an element's text is cased before
/// it is measured and painted.
///
/// Only the case keywords are supported; `full-width` and
/// `full-size-kana` are ignored and treated as `none`. `capitalize`
/// works per text node, so a word split across elements (`a<b>b</b>`)
/// gets a capital at the start of each piece.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum TextTransform {
    /// Text is left as written.
    #[default]
    None,
    /// Every character is upper-cased.
    Uppercase,
    /// Every character is lower-cased.
    Lowercase,
    /// The first letter of each word is upper-cased.
    Capitalize,
}

/// Computed `line-height`; unitless values remain multipliers so descendants
/// apply them to their own font size, while lengths and percentages inherit as lengths.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum LineHeight {
    Normal,
    Number(f32),
    Length(f32),
}

impl LineHeight {
    fn parse(value: &str, context: &LengthContext, font_size: f32) -> Option<Self> {
        let value = value.trim().to_ascii_lowercase();
        if value == "normal" {
            return Some(Self::Normal);
        }
        if let Ok(number) = value.parse::<f32>() {
            return (number.is_finite() && number >= 0.0).then_some(Self::Number(number));
        }
        if let Some(percent) = parse_percentage(&value) {
            let px = font_size * percent / 100.0;
            return (px.is_finite() && px >= 0.0).then_some(Self::Length(px));
        }
        let px = parse_length(&value)?.to_px(context);
        (px.is_finite() && px >= 0.0).then_some(Self::Length(px))
    }

    pub(crate) fn used(self, font_size: f32, normal: f32) -> f32 {
        match self {
            Self::Normal => normal,
            Self::Number(n) => font_size * n,
            Self::Length(px) => px,
        }
    }
}

impl TextTransform {
    /// Parses a `text-transform` value, returning `None` for unsupported
    /// or invalid values.
    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "none" => Some(Self::None),
            "uppercase" => Some(Self::Uppercase),
            "lowercase" => Some(Self::Lowercase),
            "capitalize" => Some(Self::Capitalize),
            _ => None,
        }
    }

    /// Returns `text` with this transform applied.
    pub(crate) fn apply(self, text: &str) -> String {
        match self {
            Self::None => text.to_owned(),
            Self::Uppercase => text.to_uppercase(),
            Self::Lowercase => text.to_lowercase(),
            Self::Capitalize => {
                let mut out = String::with_capacity(text.len());
                let mut at_word_start = true;
                for character in text.chars() {
                    if at_word_start && character.is_alphanumeric() {
                        out.extend(character.to_uppercase());
                    } else {
                        out.push(character);
                    }
                    at_word_start = character.is_whitespace();
                }
                out
            }
        }
    }
}

/// The `text-align` property: how inline content is placed within each line
/// box (CSS Text 3 §7.1).
///
/// Text is always left-to-right today, so `start` behaves as `left` and `end`
/// as `right`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum TextAlign {
    /// Aligned to the start edge of the line box.
    #[default]
    Start,
    /// Aligned to the end edge of the line box.
    End,
    /// Aligned to the left edge.
    Left,
    /// Aligned to the right edge.
    Right,
    /// Centered in the line box.
    Center,
    /// Stretched to both edges by widening spaces, except on the last line
    /// and lines ending in a forced break, which are start-aligned.
    Justify,
    /// Like `start` or `end`, but resolved against the parent's direction;
    /// replaced with the parent's value when styles are resolved.
    MatchParent,
}

impl TextAlign {
    /// Parses a `text-align` value, returning `None` for invalid values.
    /// The legacy `-webkit-` keywords map to their plain equivalents.
    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "start" | "initial" | "unset" => Some(Self::Start),
            "end" => Some(Self::End),
            "left" | "-webkit-left" => Some(Self::Left),
            "right" | "-webkit-right" => Some(Self::Right),
            "center" | "-webkit-center" => Some(Self::Center),
            "justify" => Some(Self::Justify),
            "match-parent" | "inherit" => Some(Self::MatchParent),
            _ => None,
        }
    }

    /// The text-align computed for an element whose cascaded value is
    /// `value`, falling back to `parent` for `match-parent` and to `start`
    /// for invalid values.
    pub(crate) fn compute(value: Option<&str>, parent: impl FnOnce() -> Self) -> Self {
        match value.map(|value| (value, Self::parse(value.trim()))) {
            None => Self::Start,
            Some((_, Some(Self::MatchParent))) => parent(),
            Some((_, Some(align))) => align,
            Some((value, None)) => {
                log::debug!("ignoring invalid text-align value {value:?}; using start");
                Self::Start
            }
        }
    }

    /// The fraction of a line's free space placed before its content in a
    /// left-to-right line: 0 for left, 0.5 for center, 1 for right.
    pub(crate) fn offset_fraction(self) -> f32 {
        match self {
            Self::Start | Self::Left | Self::Justify | Self::MatchParent => 0.0,
            Self::Center => 0.5,
            Self::End | Self::Right => 1.0,
        }
    }
}

/// The computed `box-sizing`: which box `width`, `height`, and their `min-*`
/// and `max-*` limits size (CSS Box Sizing 3 §4.1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum BoxSizing {
    /// The sizes are the content box's.
    #[default]
    ContentBox,
    /// The sizes are the border box's, so padding and borders is subtracted to get the content size.
    BorderBox,
}

impl BoxSizing {
    /// Parses a `box-sizing` value, ignoring ASCII case.
    fn parse(value: &str) -> Option<Self> {
        if value.eq_ignore_ascii_case("content-box") {
            Some(Self::ContentBox)
        } else if value.eq_ignore_ascii_case("border-box") {
            Some(Self::BorderBox)
        } else {
            None
        }
    }
}

/// The values layout reads from one element's computed style.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ComputedStyle {
    pub display: Display,
    pub margin: Sides<Dimension>,
    pub padding: Sides<Dimension>,
    /// `width`, measuring the box chosen by `box_sizing`; read the content
    /// width through [`ComputedStyle::content_width`]. Non-replaced boxes
    /// support the `min-content`, `max-content`, and `fit-content` keywords.
    pub width: Dimension,
    /// `height`, as for `width`; see [`ComputedStyle::content_height`].
    pub height: Dimension,
    /// Which box `width`, `height`, and their limits measure.
    pub box_sizing: BoxSizing,
    /// `min-width`; replaced elements and flex items honor it so far.
    pub min_width: Dimension,
    /// `min-height`; replaced elements and flex items honor it so far.
    pub min_height: Dimension,
    /// Whether `min-width` was left at the flex item's automatic minimum.
    pub min_width_auto: bool,
    /// Whether `min-height` was left at the flex item's automatic minimum.
    pub min_height_auto: bool,
    /// `max-width`, with `none` stored as [`Dimension::Auto`]; only replaced
    /// elements honor it so far.
    pub max_width: Dimension,
    /// `max-height`, as for `max_width`.
    pub max_height: Dimension,
    pub font_size: f32,
    pub line_height: LineHeight,
    /// The computed `font-family` list, unquoted, in order of preference.
    /// Empty means the user agent's default font.
    pub font_families: Rc<[String]>,
    /// The computed `font-weight`, from 1 to 1000.
    pub font_weight: u16,
    /// Whether the computed `font-style` is italic or oblique.
    pub font_italic: bool,
    /// The face [`crate::FontMetrics::select_font`] chose for the three
    /// properties above. Set by the style map, which knows the fonts.
    pub font: crate::FontId,
    pub color: CssColor,
    pub text_transform: TextTransform,
    pub text_align: TextAlign,
    pub background_color: Option<CssColor>,
    pub borders: crate::BoxStyle,
    pub border_radius: BorderRadius,
    /// The computed `box-shadow` list, first (topmost) shadow first.
    pub box_shadow: Arc<[BoxShadow]>,
    pub flex_column: bool,
    pub flex_grow: f32,
    /// Flex base size before free-space distribution; `auto` uses the
    /// corresponding main-size property or the item's intrinsic size.
    pub flex_basis: Dimension,
    /// The factor used to distribute negative free space among flex items.
    pub flex_shrink: f32,
    pub gap: f32,
    /// The text a `::before` / `::after` box holds, from the `content`
    /// property. `None` means no box is generated. Ignored on elements.
    pub content: Option<String>,
    pub position: Position,
    /// The computed `float`; always [`Float::None`] for absolutely
    /// positioned boxes (CSS 2.2 §9.7).
    pub float: Float,
    /// The `top`, `right`, `bottom`, and `left` insets; `auto` by default.
    pub inset: Sides<Dimension>,
    /// Whether the specified `display` was inline-level before floating or
    /// absolute positioning blockified it. It decides an absolutely
    /// positioned box's static position inside a line.
    pub blockified_from_inline: bool,
}

impl ComputedStyle {
    /// Whether the box is taken out of flow by `position: absolute|fixed`.
    pub(crate) fn is_absolutely_positioned(&self) -> bool {
        matches!(self.position, Position::Absolute | Position::Fixed)
    }

    /// Whether the box is floated.
    pub(crate) fn is_floated(&self) -> bool {
        self.float != Float::None
    }

    /// Whether the box is floated or absolutely positioned.
    pub(crate) fn is_out_of_flow(&self) -> bool {
        self.is_floated() || self.is_absolutely_positioned()
    }

    /// Whether `position` is anything but `static`, which makes the box the
    /// containing block of absolutely positioned descendants.
    pub(crate) fn is_positioned(&self) -> bool {
        self.position != Position::Static
    }
}

/// Parses a `content` value into the text a generated box holds.
///
/// Only quoted strings are supported; several strings are joined
/// (`"a" "b"` is `ab`). `none`, `normal`, and values with nothing
/// supported (`attr()`, counters, quotes, images) return `None`, so no box
/// is generated. Unsupported parts beside strings are skipped.
pub(crate) fn parse_content(value: &str) -> Option<String> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("none") || value.eq_ignore_ascii_case("normal") {
        return None;
    }
    let mut text = String::new();
    let mut found = false;
    let mut chars = value.chars().peekable();
    while let Some(character) = chars.next() {
        if character != '"' && character != '\'' {
            continue;
        }
        let quote = character;
        found = true;
        while let Some(character) = chars.next() {
            match character {
                c if c == quote => break,
                '\\' => match chars.peek().copied() {
                    // An escaped newline continues the string.
                    Some('\n') => {
                        chars.next();
                    }
                    Some(hex) if hex.is_ascii_hexdigit() => {
                        let mut code = 0_u32;
                        for _ in 0..6 {
                            match chars.peek().and_then(|digit| digit.to_digit(16)) {
                                Some(digit) => {
                                    code = code * 16 + digit;
                                    chars.next();
                                }
                                None => break,
                            }
                        }
                        // One white space after a hex escape is part of it.
                        if chars.peek().is_some_and(|next| next.is_ascii_whitespace()) {
                            chars.next();
                        }
                        text.push(
                            char::from_u32(code)
                                .filter(|c| *c != '\0')
                                .unwrap_or('\u{FFFD}'),
                        );
                    }
                    Some(escaped) => {
                        text.push(escaped);
                        chars.next();
                    }
                    None => {}
                },
                c => text.push(c),
            }
        }
    }
    if !found {
        log::debug!("ignoring unsupported content value {value:?}");
    }
    found.then_some(text)
}

impl ComputedStyle {
    /// The style of a box with no element: initial values for every property,
    /// with inherited properties (`color`, `font-size`, `text-transform`, and
    /// `text-align`) taken from
    /// `parent` when given.
    ///
    /// Anonymous boxes use this (CSS 2.2 §9.2.1.1), as does the initial
    /// containing block with no parent.
    pub(crate) fn anonymous(parent: Option<&Self>) -> Self {
        let zero = Sides::from_array([Dimension::Px(0.0); 4]);
        Self {
            display: Display::Box {
                outside: DisplayOutside::Block,
                inside: DisplayInside::Flow,
            },
            margin: zero,
            padding: zero,
            width: Dimension::Auto,
            height: Dimension::Auto,
            box_sizing: BoxSizing::ContentBox,
            min_width: Dimension::Px(0.0),
            min_height: Dimension::Px(0.0),
            min_width_auto: true,
            min_height_auto: true,
            max_width: Dimension::Auto,
            max_height: Dimension::Auto,
            font_size: parent.map_or(css::values::MEDIUM_FONT_SIZE, |parent| parent.font_size),
            line_height: parent.map_or(LineHeight::Normal, |parent| parent.line_height),
            font_families: parent.map_or_else(
                || Rc::from(Vec::new()),
                |parent| Rc::clone(&parent.font_families),
            ),
            font_weight: parent.map_or(400, |parent| parent.font_weight),
            font_italic: parent.is_some_and(|parent| parent.font_italic),
            font: parent.map_or(crate::FontId::DEFAULT, |parent| parent.font),
            color: parent.map_or_else(
                || {
                    css::user_agent_default_color("color")
                        .expect("user-agent stylesheet must define html color")
                },
                |parent| parent.color,
            ),
            text_transform: parent.map_or(TextTransform::None, |parent| parent.text_transform),
            text_align: parent.map_or(TextAlign::Start, |parent| parent.text_align),
            background_color: None,
            content: None,
            borders: crate::BoxStyle::default(),
            border_radius: BorderRadius::default(),
            box_shadow: Arc::from(Vec::new()),
            flex_column: false,
            flex_grow: 0.0,
            flex_basis: Dimension::Auto,
            flex_shrink: 1.0,
            gap: 0.0,
            position: Position::Static,
            float: Float::None,
            inset: Sides::from_array([Dimension::Auto; 4]),
            blockified_from_inline: false,
        }
    }

    /// Builds the style of an element from its computed declarations.
    ///
    /// `root_font_size` resolves `rem`; `viewport` resolves viewport units.
    /// `parent_text_align` supplies the parent's computed `text-align` for
    /// `match-parent`; it is only called when needed.
    pub(crate) fn from_declarations(
        declarations: &[Declaration],
        root_font_size: f32,
        viewport: crate::Size,
        parent_text_align: impl FnOnce() -> TextAlign,
    ) -> Self {
        let get = |property: &str| {
            declarations
                .iter()
                .find(|declaration| declaration.property == property)
                .map(|declaration| declaration.value.trim())
        };
        let mut style = Self::anonymous(None);
        style.content = get("content").and_then(parse_content);
        style.display = match get("display") {
            Some(value) => Display::parse(value).unwrap_or_else(|| {
                log::debug!("ignoring invalid display value {value:?}; using inline");
                Display::INLINE
            }),
            None => Display::INLINE,
        };
        if let Some(value) = get("font-size") {
            match css::values::compute_font_size(value, root_font_size, root_font_size) {
                Some(size) => style.font_size = size,
                None => log::debug!(
                    "ignoring invalid font-size value {value:?}; using {}px",
                    style.font_size
                ),
            }
        }
        if let Some(value) = get("line-height")
            && let Some(line_height) = LineHeight::parse(
                value,
                &LengthContext {
                    font_size: style.font_size,
                    root_font_size,
                    viewport_width: viewport.width,
                    viewport_height: viewport.height,
                },
                style.font_size,
            )
        {
            style.line_height = line_height;
        }
        if let Some(value) = get("font-family") {
            style.font_families = css::parse_font_family_list(value).into();
        }
        if let Some(value) = get("font-weight") {
            match css::fonts::parse_font_weight(value) {
                Some(weight) => style.font_weight = weight,
                None => log::debug!("ignoring invalid font-weight value {value:?}; using 400"),
            }
        }
        style.font_italic = get("font-style").is_some_and(css::fonts::is_italic);
        if let Some(value) = get("color") {
            match css::parse_color(value) {
                Some(color) => style.color = color,
                // Transparent text is laid out but paints nothing, as when a
                // CSS animation fades text in from `transparent`.
                None if value.eq_ignore_ascii_case("transparent") => {
                    style.color = CssColor {
                        a: 0,
                        ..style.color
                    };
                }
                None if is_no_color_keyword(value) => {}
                None => log::debug!("ignoring invalid color value {value:?}; using the default"),
            }
        }
        if let Some(value) = get("text-transform") {
            match TextTransform::parse(value) {
                Some(transform) => style.text_transform = transform,
                None => log::debug!("ignoring unsupported text-transform value {value:?}"),
            }
        }
        style.text_align = TextAlign::compute(get("text-align"), parent_text_align);
        if let Some(value) = get("position") {
            match Position::parse(value) {
                Some(position) => style.position = position,
                None if is_css_wide_keyword(value) => {}
                None => log::debug!("ignoring invalid position value {value:?}; using static"),
            }
        }
        if let Some(value) = get("float") {
            match Float::parse(value) {
                Some(float) => style.float = float,
                None if is_css_wide_keyword(value) => {}
                None => log::debug!("ignoring invalid float value {value:?}; using none"),
            }
        }
        // CSS 2.2 §9.7: absolute positioning wins over floating, and both
        // blockify the display type. `display: contents` generates no box,
        // so there is nothing to blockify.
        if style.is_absolutely_positioned() {
            style.float = Float::None;
        }
        if style.is_out_of_flow()
            && let Display::Box { outside, inside } = style.display
        {
            style.blockified_from_inline = outside == DisplayOutside::Inline;
            style.display = Display::Box {
                outside: DisplayOutside::Block,
                inside,
            };
        }
        if let Some(value) = get("background-color") {
            style.background_color = if value.eq_ignore_ascii_case("currentcolor") {
                Some(style.color)
            } else {
                css::parse_color(value)
            };
            if style.background_color.is_none() && !is_no_color_keyword(value) {
                log::debug!("ignoring invalid background-color value {value:?}; using transparent");
            }
        }
        style.flex_column =
            get("flex-direction").is_some_and(|value| value.eq_ignore_ascii_case("column"));
        style.flex_grow = get("flex-grow")
            .and_then(|value| value.parse::<f32>().ok())
            .filter(|value| value.is_finite() && *value >= 0.0)
            .unwrap_or(0.0);
        style.flex_shrink = get("flex-shrink")
            .and_then(|value| value.parse::<f32>().ok())
            .filter(|value| value.is_finite() && *value >= 0.0)
            .unwrap_or(1.0);

        let context = LengthContext {
            font_size: style.font_size,
            root_font_size,
            viewport_width: viewport.width,
            viewport_height: viewport.height,
        };
        let dimension = |property: &str, allow_auto: bool, allow_negative: bool| {
            let value = get(property)?;
            if property == "width" {
                match value.to_ascii_lowercase().as_str() {
                    "min-content" => return Some(Dimension::MinContent),
                    "max-content" => return Some(Dimension::MaxContent),
                    "fit-content" => return Some(Dimension::FitContent),
                    _ => {}
                }
            }
            if value.eq_ignore_ascii_case("auto") {
                if !allow_auto {
                    log::debug!("ignoring invalid {property} value {value:?}; using 0");
                }
                return allow_auto.then_some(Dimension::Auto);
            }
            let (parsed, amount) = match parse_percentage(value) {
                Some(percent) => (Dimension::Percent(percent), percent),
                None => {
                    let Some(length) = parse_length(value) else {
                        log::debug!(
                            "ignoring invalid {property} value {value:?}; using the initial value"
                        );
                        return None;
                    };
                    let px = length.to_px(&context);
                    (Dimension::Px(px), px)
                }
            };
            if !allow_negative && amount < 0.0 {
                log::debug!(
                    "ignoring negative {property} value {value:?}; using the initial value"
                );
                return None;
            }
            Some(parsed)
        };
        style.flex_basis = dimension("flex-basis", true, false).unwrap_or(Dimension::Auto);
        let side = |prefix: &str, allow_auto: bool, allow_negative: bool| {
            Sides::from_array(["top", "right", "bottom", "left"].map(|side| {
                dimension(&format!("{prefix}-{side}"), allow_auto, allow_negative)
                    .unwrap_or(Dimension::Px(0.0))
            }))
        };
        style.margin = side("margin", true, true);
        style.inset = Sides::from_array(
            ["top", "right", "bottom", "left"]
                .map(|property| dimension(property, true, true).unwrap_or(Dimension::Auto)),
        );
        style.padding = side("padding", false, false);
        style.width = dimension("width", true, false).unwrap_or(Dimension::Auto);
        style.height = dimension("height", true, false).unwrap_or(Dimension::Auto);
        // Store the numeric fallback here and retain whether the flex
        // automatic minimum applies in the flags below. `max-*: none` is
        // stored as `Auto` (no maximum).
        style.min_width = dimension("min-width", true, false)
            .filter(|value| *value != Dimension::Auto)
            .unwrap_or(Dimension::Px(0.0));
        style.min_height = dimension("min-height", true, false)
            .filter(|value| *value != Dimension::Auto)
            .unwrap_or(Dimension::Px(0.0));
        style.min_width_auto =
            get("min-width").is_none_or(|value| value.eq_ignore_ascii_case("auto"));
        style.min_height_auto =
            get("min-height").is_none_or(|value| value.eq_ignore_ascii_case("auto"));
        let max = |property: &str| {
            if get(property).is_some_and(|value| value.eq_ignore_ascii_case("none")) {
                return Dimension::Auto;
            }
            dimension(property, false, false).unwrap_or(Dimension::Auto)
        };
        style.max_width = max("max-width");
        style.max_height = max("max-height");
        style.border_radius = Corners::from_array(
            ["top-left", "top-right", "bottom-right", "bottom-left"].map(|corner| {
                let property = format!("border-{corner}-radius");
                get(&property)
                    .and_then(|value| {
                        let radius = parse_corner_radius(value, &context);
                        if radius.is_none() && !is_css_wide_keyword(value) {
                            log::debug!("ignoring invalid {property} value {value:?}; using 0");
                        }
                        radius
                    })
                    .unwrap_or_default()
            }),
        );
        let sides = ["top", "right", "bottom", "left"];
        style.borders.border_style = Sides::from_array(sides.map(|side| {
            get(&format!("border-{side}-style"))
                .unwrap_or("none")
                .to_ascii_lowercase()
        }));
        style.borders.border_width = Sides::from_array(sides.map(|side| {
            let line = get(&format!("border-{side}-style")).unwrap_or("none");
            if !matches!(
                line.to_ascii_lowercase().as_str(),
                "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset" | "outset"
            ) {
                return 0.0;
            }
            let value = get(&format!("border-{side}-width")).unwrap_or("medium");
            match value.to_ascii_lowercase().as_str() {
                "thin" => 1.0,
                "thick" => 5.0,
                "medium" | "initial" | "unset" => 3.0,
                _ => parse_length(value)
                    .map(|v| v.to_px(&context))
                    .filter(|v| v.is_finite() && *v >= 0.0)
                    .unwrap_or(3.0),
            }
        }));
        style.borders.border_color = Sides::from_array(sides.map(|side| {
            get(&format!("border-{side}-color"))
                .and_then(css::parse_color)
                .unwrap_or(style.color)
        }));
        style.gap = get("gap")
            .and_then(|value| parse_length(value).map(|length| length.to_px(&context)))
            .filter(|value| value.is_finite() && *value >= 0.0)
            .unwrap_or(0.0);
        if let Some(value) = get("box-sizing") {
            match BoxSizing::parse(value) {
                Some(sizing) => style.box_sizing = sizing,
                None if is_css_wide_keyword(value) => {}
                None => {
                    log::debug!("ignoring invalid box-sizing value {value:?}; using content-box")
                }
            }
        }
        if let Some(value) = get("box-shadow") {
            match parse_box_shadow(value, &context, style.color) {
                Some(shadows) => style.box_shadow = shadows.into(),
                None if is_css_wide_keyword(value) => {}
                None => log::debug!("ignoring invalid box-shadow value {value:?}; using none"),
            }
        }
        style
    }

    /// Combined padding and border edges in CSS pixels for layout arithmetic.
    pub(crate) fn outer_edges(&self, basis: Option<f32>) -> crate::Edges {
        let p = self.padding.map(|v| v.resolve_or_zero(basis));
        let b = self.borders.border_width;
        crate::Edges {
            top: p.top + b.top,
            right: p.right + b.right,
            bottom: p.bottom + b.bottom,
            left: p.left + b.left,
        }
    }

    /// The used content-box width for a `width` (or `min-width` /
    /// `max-width`) value `size` resolved against `cb_width`.
    ///
    /// With `box-sizing: border-box` the horizontal padding and borders are subtracted,
    /// never going below zero. Returns `None` when `size` is `auto` or an
    /// unresolvable percentage.
    pub(crate) fn content_width_for(&self, size: Dimension, cb_width: Option<f32>) -> Option<f32> {
        let width = size.resolve(cb_width)?;
        Some(match self.box_sizing {
            BoxSizing::ContentBox => width,
            BoxSizing::BorderBox => (width - self.outer_edges(cb_width).horizontal()).max(0.0),
        })
    }

    /// The used content-box height for a `height` (or `min-height` /
    /// `max-height`) value `size` resolved against `cb_height`.
    ///
    /// Padding percentages resolve against `cb_width`, as in CSS. With
    /// `box-sizing: border-box` the vertical padding and borders are subtracted, never
    /// going below zero.
    pub(crate) fn content_height_for(
        &self,
        size: Dimension,
        cb_width: Option<f32>,
        cb_height: Option<f32>,
    ) -> Option<f32> {
        let height = size.resolve(cb_height)?;
        Some(match self.box_sizing {
            BoxSizing::ContentBox => height,
            BoxSizing::BorderBox => (height - self.outer_edges(cb_width).vertical()).max(0.0),
        })
    }

    /// The used content-box width from `width`; see
    /// [`ComputedStyle::content_width_for`].
    pub(crate) fn content_width(&self, cb_width: Option<f32>) -> Option<f32> {
        self.content_width_for(self.width, cb_width)
    }

    /// The used content-box height from `height`; see
    /// [`ComputedStyle::content_height_for`].
    pub(crate) fn content_height(
        &self,
        cb_width: Option<f32>,
        cb_height: Option<f32>,
    ) -> Option<f32> {
        self.content_height_for(self.height, cb_width, cb_height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_strings_are_unquoted_and_unescaped() {
        assert_eq!(parse_content("\"SUCCESS\"").as_deref(), Some("SUCCESS"));
        assert_eq!(parse_content("'it\\'s'").as_deref(), Some("it's"));
        assert_eq!(parse_content("\"\\201C x\"").as_deref(), Some("\u{201C}x"));
        assert_eq!(parse_content("\"a\" \"b\"").as_deref(), Some("ab"));
        assert_eq!(parse_content("\"\"").as_deref(), Some(""));
        assert_eq!(parse_content("none"), None);
        assert_eq!(parse_content("NORMAL"), None);
        assert_eq!(parse_content("attr(title)"), None);
        assert_eq!(parse_content("open-quote \"x\"").as_deref(), Some("x"));
    }
    use crate::Size;

    fn style(declarations: &[(&str, &str)]) -> ComputedStyle {
        let declarations = declarations
            .iter()
            .map(|(property, value)| Declaration {
                property: (*property).into(),
                value: (*value).into(),
            })
            .collect::<Vec<_>>();
        ComputedStyle::from_declarations(
            &declarations,
            16.0,
            Size {
                width: 800.0,
                height: 600.0,
            },
            || TextAlign::Right,
        )
    }

    #[test]
    fn text_align_parses_keywords_and_match_parent() {
        assert_eq!(style(&[]).text_align, TextAlign::Start);
        assert_eq!(
            style(&[("text-align", "CENTER")]).text_align,
            TextAlign::Center
        );
        assert_eq!(
            style(&[("text-align", "-webkit-right")]).text_align,
            TextAlign::Right
        );
        assert_eq!(
            style(&[("text-align", "justify")]).text_align,
            TextAlign::Justify
        );
        assert_eq!(
            style(&[("text-align", "match-parent")]).text_align,
            TextAlign::Right
        );
        assert_eq!(
            style(&[("text-align", "middle")]).text_align,
            TextAlign::Start
        );
    }

    #[test]
    fn display_defaults_to_inline_and_parses_both_syntaxes() {
        assert_eq!(style(&[]).display, Display::INLINE);
        assert_eq!(
            style(&[("display", "inline-block")]).display,
            Display::Box {
                outside: DisplayOutside::Inline,
                inside: DisplayInside::FlowRoot
            }
        );
        assert_eq!(
            style(&[("display", "block flow-root")]).display,
            Display::Box {
                outside: DisplayOutside::Block,
                inside: DisplayInside::FlowRoot
            }
        );
        assert_eq!(style(&[("display", "bogus")]).display, Display::INLINE);
    }

    #[test]
    fn display_preserves_independent_layout_modes() {
        assert!(matches!(
            style(&[("display", "flex")]).display,
            Display::Box {
                inside: DisplayInside::Flex,
                ..
            }
        ));
        assert!(matches!(
            style(&[("display", "grid")]).display,
            Display::Box {
                inside: DisplayInside::Grid,
                ..
            }
        ));
        assert!(matches!(
            style(&[("display", "table")]).display,
            Display::Box {
                inside: DisplayInside::Table,
                ..
            }
        ));
    }

    #[test]
    fn border_radius_longhands_parse_lengths_and_percentages() {
        let style = style(&[
            ("font-size", "10px"),
            ("border-top-left-radius", "3px"),
            ("border-top-right-radius", "1em 50%"),
            ("border-bottom-right-radius", "-2px"),
            ("border-bottom-left-radius", "inherit"),
        ]);
        let radius = style.border_radius;
        assert_eq!(
            radius.top_left,
            CornerRadius {
                horizontal: LengthPercentage::Px(3.0),
                vertical: LengthPercentage::Px(3.0)
            }
        );
        assert_eq!(
            radius.top_right,
            CornerRadius {
                horizontal: LengthPercentage::Px(10.0),
                vertical: LengthPercentage::Percent(50.0)
            }
        );
        assert_eq!(radius.bottom_right, CornerRadius::default());
        assert_eq!(radius.bottom_left, CornerRadius::default());
    }

    #[test]
    fn position_float_and_insets_parse_and_blockify() {
        let plain = style(&[]);
        assert_eq!(plain.position, Position::Static);
        assert_eq!(plain.float, Float::None);
        assert_eq!(plain.inset.top, Dimension::Auto);

        let absolute = style(&[
            ("position", "ABSOLUTE"),
            ("float", "left"),
            ("top", "-5px"),
            ("left", "10%"),
        ]);
        assert_eq!(absolute.position, Position::Absolute);
        assert_eq!(absolute.float, Float::None);
        assert_eq!(absolute.inset.top, Dimension::Px(-5.0));
        assert_eq!(absolute.inset.left, Dimension::Percent(10.0));
        assert_eq!(absolute.inset.right, Dimension::Auto);
        assert!(absolute.blockified_from_inline);
        assert!(matches!(
            absolute.display,
            Display::Box {
                outside: DisplayOutside::Block,
                ..
            }
        ));

        let floated = style(&[("display", "inline-block"), ("float", "inline-end")]);
        assert_eq!(floated.float, Float::Right);
        assert_eq!(
            floated.display,
            Display::Box {
                outside: DisplayOutside::Block,
                inside: DisplayInside::FlowRoot
            }
        );
        assert_eq!(style(&[("position", "bogus")]).position, Position::Static);
        assert_eq!(style(&[("float", "middle")]).float, Float::None);
        assert_eq!(
            style(&[("position", "-webkit-sticky")]).position,
            Position::Sticky
        );
    }

    #[test]
    fn em_lengths_use_the_elements_own_font_size() {
        let style = style(&[("font-size", "32px"), ("margin-top", "0.5em")]);
        assert_eq!(style.margin.top, Dimension::Px(16.0));
    }

    #[test]
    fn negative_padding_and_sizes_are_invalid_but_negative_margins_are_not() {
        let style = style(&[
            ("padding-left", "-4px"),
            ("width", "-1px"),
            ("margin-left", "-4px"),
        ]);
        assert_eq!(style.padding.left, Dimension::Px(0.0));
        assert_eq!(style.width, Dimension::Auto);
        assert_eq!(style.margin.left, Dimension::Px(-4.0));
    }
}
