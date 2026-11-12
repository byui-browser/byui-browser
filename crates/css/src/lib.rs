//! CSS parsing, cascade, inheritance, and computed styles.
//!
//! Stylesheets are tokenized per CSS Syntax Level 3 ([`tokenizer`]) and parsed
//! from tokens. The parser is intentionally forgiving, like a browser parser:
//! it keeps valid rules when a neighboring rule is malformed and reports
//! malformed pieces as [`ParseError`] values.
//!
//! [`style_document`] runs the cascade. It compiles each stylesheet's
//! selectors once, indexes rules by their rightmost ID, class, attribute, or
//! tag, and only tests an element against rules that could match it.

#![forbid(unsafe_code)]

mod cascade;
mod color;
mod computed;
mod matching;
mod parser;
mod rule_index;
pub mod selector;
pub mod shorthands;
mod specificity;
mod stylesheet;
pub mod tokenizer;
mod user_agent;
pub mod values;

pub use cascade::style_document;
pub use color::{CssColor, parse_color};
pub use computed::{ComputedStyles, PseudoElement};
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
