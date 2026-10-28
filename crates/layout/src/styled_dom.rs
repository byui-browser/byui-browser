//! The layout-facing projection of the document tree.

use html::{HTMLDocument, NodeID, NodeKind};

/// The layout-facing projection of an HTML document.
///
/// Until the CSS stage exists, this holds one shared arena index for each
/// visible HTML element and no computed style information. Document
/// metadata (`html`, `head`, `link`, `meta`, `script`, `style`, `title`) is
/// omitted, and `body` is kept only when it owns direct non-whitespace text;
/// see [`StyledDom::from_html_document`] for the rule.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StyledDom {
    /// HTML arena indices represented as layout nodes.
    pub nodes: Vec<NodeID>,
}

impl StyledDom {
    /// Creates the first layout slice's projection: visible HTML elements are
    /// represented as blocks while document metadata is omitted. A body is
    /// retained when it owns direct text so that text-only documents render;
    /// CSS will eventually provide the style information.
    pub fn from_html_document(document: &HTMLDocument) -> Self {
        let nodes = document
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                let NodeKind::Element(element) = &node.kind else {
                    return None;
                };
                if matches!(
                    element.name.as_str(),
                    "head" | "html" | "link" | "meta" | "script" | "style" | "title"
                ) {
                    return None;
                }
                if element.name == "body"
                    && !node.children.iter().any(|child| {
                        document
                            .node(*child)
                            .is_some_and(|child| {
                                matches!(&child.kind, NodeKind::Text(text) if !text.trim().is_empty())
                            })
                    })
                {
                    return None;
                }
                Some(NodeID::new(index))
            })
            .collect();
        Self { nodes }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_elements_do_not_consume_layout_blocks() {
        let document = html::parse_raw_html(
            "<html><head><link rel=\"stylesheet\"></head><body><main>Welcome</main><script></script></body></html>"
                .to_owned(),
        );

        let styled = StyledDom::from_html_document(&document);

        assert_eq!(styled.nodes.len(), 1);
        assert!(matches!(
            document.node(styled.nodes[0]),
            Some(html::Node {
                kind: html::NodeKind::Element(element),
                ..
            }) if element.name == "main"
        ));
    }

    #[test]
    fn body_text_has_a_layout_block() {
        let document = html::parse_raw_html("<body>Hello</body>".to_owned());

        let styled = StyledDom::from_html_document(&document);

        assert_eq!(styled.nodes.len(), 1);
        assert!(matches!(
            document.node(styled.nodes[0]),
            Some(html::Node {
                kind: html::NodeKind::Element(element),
                ..
            }) if element.name == "body"
        ));
    }
}
