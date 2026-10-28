//! Typed computed values for the properties layout consumes.
//!
//! [`css::style_document`] produces cascaded, inherited declarations as text.
//! This module parses the properties layout supports today: `display`,
//! `margin-*`, `padding-*`, `width`, `height`, `font-size`, `color`, and
//! `background-color`. Every other property behaves as its initial value; for
//! example borders are absent, `line-height` is `normal`, and `white-space`
//! is `normal`. A missing or unparsable value also falls back to the initial
//! value. Lengths are resolved to CSS pixels here; percentages are kept
//! because they resolve against the containing block during layout.

use css::values::{LengthContext, parse_length, parse_percentage};
use css::{CssColor, Declaration};

use crate::Sides;

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
/// Only flow layout is implemented. `table`, `flex`, `grid`, and the other
/// independent layouts are laid out as `flow-root`, which keeps their contents
/// in a formatting context of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DisplayInside {
    Flow,
    FlowRoot,
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
            "inline-block" | "inline-table" | "inline-flex" | "inline-grid" => {
                Some(inline(DisplayInside::FlowRoot))
            }
            "flow-root" | "table" | "flex" | "grid" | "table-row-group" | "table-header-group"
            | "table-footer-group" | "table-row" | "table-cell" | "table-column-group"
            | "table-column" | "table-caption" => Some(block(DisplayInside::FlowRoot)),
            _ => None,
        };
        if single.is_some() {
            return single;
        }
        let mut outside = None;
        let mut inside = None;
        for keyword in value.split_ascii_whitespace() {
            match keyword {
                "block" if outside.is_none() => outside = Some(DisplayOutside::Block),
                "inline" if outside.is_none() => outside = Some(DisplayOutside::Inline),
                "flow" if inside.is_none() => inside = Some(DisplayInside::Flow),
                "flow-root" | "table" | "flex" | "grid" if inside.is_none() => {
                    inside = Some(DisplayInside::FlowRoot);
                }
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

/// A `<length-percentage>` or `auto`, with lengths resolved to CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Dimension {
    Auto,
    Px(f32),
    Percent(f32),
}

impl Dimension {
    /// Resolves against `basis`, returning `None` for `auto` or for a
    /// percentage whose basis is indefinite.
    pub(crate) fn resolve(self, basis: Option<f32>) -> Option<f32> {
        match self {
            Self::Auto => None,
            Self::Px(px) => Some(px),
            Self::Percent(percent) => basis.map(|basis| basis * percent / 100.0),
        }
    }

    /// Resolves treating `auto` and indefinite percentages as zero.
    pub(crate) fn resolve_or_zero(self, basis: Option<f32>) -> f32 {
        self.resolve(basis).unwrap_or(0.0)
    }
}

/// The values layout reads from one element's computed style.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ComputedStyle {
    pub display: Display,
    pub margin: Sides<Dimension>,
    pub padding: Sides<Dimension>,
    pub width: Dimension,
    pub height: Dimension,
    pub font_size: f32,
    pub color: CssColor,
    pub background_color: Option<CssColor>,
}

impl ComputedStyle {
    /// The style of a box with no element: initial values for every property,
    /// with inherited properties (`color` and `font-size`) taken from
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
            font_size: parent.map_or(css::values::MEDIUM_FONT_SIZE, |parent| parent.font_size),
            color: parent.map_or_else(
                || {
                    css::user_agent_default_color("color")
                        .expect("user-agent stylesheet must define html color")
                },
                |parent| parent.color,
            ),
            background_color: None,
        }
    }

    /// Builds the style of an element from its computed declarations.
    ///
    /// `root_font_size` resolves `rem`; `viewport` resolves viewport units.
    pub(crate) fn from_declarations(
        declarations: &[Declaration],
        root_font_size: f32,
        viewport: crate::Size,
    ) -> Self {
        let get = |property: &str| {
            declarations
                .iter()
                .find(|declaration| declaration.property == property)
                .map(|declaration| declaration.value.trim())
        };
        let mut style = Self::anonymous(None);
        style.display = get("display")
            .and_then(Display::parse)
            .unwrap_or(Display::INLINE);
        if let Some(size) = get("font-size")
            .and_then(|value| css::values::compute_font_size(value, root_font_size, root_font_size))
        {
            style.font_size = size;
        }
        if let Some(color) = get("color").and_then(css::parse_color) {
            style.color = color;
        }
        style.background_color = get("background-color").and_then(css::parse_color);

        let context = LengthContext {
            font_size: style.font_size,
            root_font_size,
            viewport_width: viewport.width,
            viewport_height: viewport.height,
        };
        let dimension = |property: &str, allow_auto: bool, allow_negative: bool| {
            let value = get(property)?;
            if value.eq_ignore_ascii_case("auto") {
                return allow_auto.then_some(Dimension::Auto);
            }
            let (parsed, amount) = match parse_percentage(value) {
                Some(percent) => (Dimension::Percent(percent), percent),
                None => {
                    let px = parse_length(value)?.to_px(&context);
                    (Dimension::Px(px), px)
                }
            };
            (allow_negative || amount >= 0.0).then_some(parsed)
        };
        let side = |prefix: &str, allow_auto: bool, allow_negative: bool| {
            Sides::from_array(["top", "right", "bottom", "left"].map(|side| {
                dimension(&format!("{prefix}-{side}"), allow_auto, allow_negative)
                    .unwrap_or(Dimension::Px(0.0))
            }))
        };
        style.margin = side("margin", true, true);
        style.padding = side("padding", false, false);
        style.width = dimension("width", true, false).unwrap_or(Dimension::Auto);
        style.height = dimension("height", true, false).unwrap_or(Dimension::Auto);
        style
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        )
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
