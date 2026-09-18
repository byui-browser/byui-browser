//! HTML parser and DOM construction.
//!
//! **Owning team**: HTML Team
//!
//! Responsibilities: HTML tokenizer, tree builder, DOM construction,
//! mutation observers, and basic DOM APIs.

#![forbid(unsafe_code)]

use common::ids::NodeId;

/// One token produced by the tokenizer.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Reshape the
// types, names, and module layout however your crate's public API needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// `<name>`
    StartTag(String),
    /// `</name>`
    EndTag(String),
    /// Character data between tags.
    Text(String),
}

/// A single DOM node. Real DOM representation is the HTML team's call
/// (architecture §3.2 suggests arena allocation).
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// Arena handle for this node.
    pub id: NodeId,
    /// Tag name for elements, `"#text"` for text nodes.
    pub name: String,
    /// Parent handle, `None` for the document root.
    pub parent: Option<NodeId>,
}

/// The document tree produced by [`parse`].
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Dom {
    /// Nodes in arena order; `NodeId` indexes into this.
    pub nodes: Vec<Node>,
}

/// Splits raw markup into tokens.
///
/// Currently only handles the empty document.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Reshape the
// signature however your crate's public API needs.
pub fn tokenize(input: &str) -> Vec<Token> {
    if input.is_empty() {
        return Vec::new();
    }
    todo!("TODO(html): tokenize non-empty input: {input:?}")
}

/// Parses markup into a DOM tree.
///
/// Currently only handles the empty document.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Reshape the
// signature however your crate's public API needs.
pub fn parse(input: &str) -> Dom {
    if input.is_empty() {
        return Dom::default();
    }
    todo!("TODO(html): build a tree from: {input:?}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_no_tokens() {
        assert!(tokenize("").is_empty());
    }

    #[test]
    fn empty_input_yields_empty_dom() {
        assert_eq!(parse(""), Dom::default());
    }
}
