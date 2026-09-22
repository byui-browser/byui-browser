//! HTML parsing, DOM construction, and basic DOM queries.
//!
//! **Owning team**: HTML Team

#![forbid(unsafe_code)]

mod dom;
mod parser;
mod selector;
mod tokenizer;

pub use dom::{Dom, HTMLDocument, HTMLElement, Location, Node, Token};
pub use parser::parse_raw_html;
pub use selector::Query;
pub use tokenizer::{parse, tokenize};
