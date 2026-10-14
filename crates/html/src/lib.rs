//! HTML parsing, DOM construction, and basic DOM queries.
//!
//! **Owning team**: HTML Team

#![forbid(unsafe_code)]

mod dom;
mod escape_characters;
mod parser;
mod selector;

pub use dom::{
    Attribute, DomError, ElementData, HTMLDocument, Namespace, Node, NodeId, NodeKind, SourceSpan,
};
pub use parser::parse_raw_html;
pub use selector::Query;
