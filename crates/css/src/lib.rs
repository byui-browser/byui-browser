//! CSS parsing, cascade, inheritance, and computed styles.
//!
//! The parser is intentionally forgiving, like a browser parser: it keeps
//! valid rules when a neighboring rule is malformed and reports malformed
//! pieces as [`ParseError`] values.

#![forbid(unsafe_code)]

mod cascade;
mod color;
mod computed;
mod parser;
pub mod selector;
pub mod shorthands;
mod specificity;
mod stylesheet;
mod user_agent;
pub mod values;

pub use cascade::style_document;
pub use color::{CssColor, parse_color};
pub use computed::ComputedStyles;
pub use parser::{parse_stylesheet, parse_stylesheet_with_errors};
pub use selector::{
    AttributeOperator, AttributeSelector, Combinator, ComplexSelector, CompoundSelector,
    PseudoSelector, SelectorList, SelectorParseError, SimpleSelector, TypeSelector,
    parse_selector_list,
};
pub use specificity::specificity;
pub use stylesheet::{
    AtRule, Declaration, Dom, ParseError, Rule, Selector, Specificity, Stylesheet,
};
pub use user_agent::{user_agent_default_color, user_agent_stylesheet};
