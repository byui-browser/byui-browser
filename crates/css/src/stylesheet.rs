//! Stylesheet abstract syntax: rules, declarations, and parse errors.

use html::HTMLDocument;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Specificity(pub u32, pub u32, pub u32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub property: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub selector: Selector,
    pub declarations: Vec<Declaration>,
}

/// An at-rule such as `@media`, `@import`, or `@font-face`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtRule {
    pub name: String,
    pub prelude: String,
    pub rules: Vec<Rule>,
    pub declarations: Vec<Declaration>,
    pub block: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub offset: usize,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
    pub at_rules: Vec<AtRule>,
    pub errors: Vec<ParseError>,
}

/// CSS's view of the HTML team's arena-backed DOM.
pub type Dom = HTMLDocument;
