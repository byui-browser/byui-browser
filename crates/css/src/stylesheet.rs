//! Stylesheet abstract syntax: rules, declarations, and parse errors.

use html::HTMLDocument;

/// The source text of one complex selector, such as `ul > li.active`.
///
/// Rules from the parser hold one complex selector each (a selector list in
/// the source is split into one [`Rule`] per entry), with comments removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector(pub String);

/// Selector specificity as `(ids, classes, types)`, compared lexicographically.
///
/// The second component also counts attribute selectors and pseudo-classes;
/// the third also counts pseudo-elements. See [`crate::specificity()`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Specificity(pub u32, pub u32, pub u32);

/// Environmental inputs used while resolving conditional CSS and viewport
/// relative values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StyleContext {
    /// Viewport width in CSS pixels.
    pub viewport_width: f32,
    /// Viewport height in CSS pixels.
    pub viewport_height: f32,
    /// Whether the output medium is a screen rather than print.
    pub screen: bool,
}

impl Default for StyleContext {
    fn default() -> Self {
        Self {
            viewport_width: 1024.0,
            viewport_height: 768.0,
            screen: true,
        }
    }
}

/// One `property: value` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    /// Property name. ASCII-lowercased, except custom properties (`--name`),
    /// which are case-sensitive.
    pub property: String,
    /// Value text as written, trimmed and without comments. A parsed
    /// `!important` annotation is normalized to a trailing ` !important`;
    /// in computed styles the annotation has been removed.
    pub value: String,
}

/// A style rule: one selector and its declarations.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    /// The rule's selector. The parser only produces valid selectors.
    pub selector: Selector,
    /// Declarations in source order.
    pub declarations: Vec<Declaration>,
    /// Nested style rules retained for CSS Nesting expansion.
    pub nested_rules: Vec<Rule>,
    /// At-rules nested inside this style rule.
    pub nested_at_rules: Vec<AtRule>,
}

/// An at-rule such as `@media`, `@import`, or `@font-face`.
///
/// The cascade applies conditional group rules and `@keyframes`; descriptor
/// at-rules are retained for the teams that consume fonts, pages, and layout.
#[derive(Debug, Clone, PartialEq)]
pub struct AtRule {
    /// ASCII-lowercased name without `@`, such as `media`.
    pub name: String,
    /// Text between the name and the block or `;`, without comments.
    pub prelude: String,
    /// Style rules inside conditional group rules (`@media`, `@supports`,
    /// `@layer`, `@container`, `@scope`, `@document`).
    pub rules: Vec<Rule>,
    /// Declarations inside descriptor at-rules such as `@font-face`, `@page`,
    /// and `@property`.
    pub declarations: Vec<Declaration>,
    /// At-rules nested inside this at-rule.
    pub at_rules: Vec<AtRule>,
    /// Keyframe rules inside `@keyframes` (and its `-webkit-`/`-moz-`
    /// prefixed aliases), in source order. Empty for other at-rules.
    pub keyframes: Vec<Keyframe>,
    /// The block's contents without comments, or `None` for statement
    /// at-rules such as `@import`.
    pub block: Option<String>,
}

/// One keyframe rule inside `@keyframes`, such as `from, 50% { color: red }`.
///
/// See CSS Animations 1 §3. The parser drops keyframe rules whose selector
/// list contains anything other than `from`, `to`, or a percentage between
/// `0%` and `100%`.
#[derive(Debug, Clone, PartialEq)]
pub struct Keyframe {
    /// Keyframe offsets as fractions of one iteration: `from` is `0.0`, `to`
    /// is `1.0`, and `25%` is `0.25`. One entry per selector in the list.
    pub offsets: Vec<f32>,
    /// Declarations in source order, exactly as in a style rule.
    /// `!important` declarations are kept here but ignored when animating.
    pub declarations: Vec<Declaration>,
}

/// A recoverable problem found while parsing a stylesheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// Human-readable description.
    pub message: String,
    /// Byte offset into the parsed input where the problem starts.
    pub offset: usize,
}

/// A parsed stylesheet.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Stylesheet {
    /// Original stylesheet source.
    pub source: String,
    /// Top-level style rules in source order.
    pub rules: Vec<Rule>,
    /// Top-level at-rules in source order.
    pub at_rules: Vec<AtRule>,
    /// Problems the parser recovered from, in source order.
    pub errors: Vec<ParseError>,
}

/// CSS's view of the HTML team's arena-backed DOM.
pub type Dom = HTMLDocument;