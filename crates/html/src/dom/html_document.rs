use super::{
    dom_error::DomError, dom_token_list::DOMTokenList, element_data::ElementData,
    namespace::Namespace, node::Node, node_id::NodeID, node_kind::NodeKind,
    source_span::SourceSpan,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HTMLDocument {
    pub nodes: Vec<Node>,
    pub root: NodeID,
}

impl Default for HTMLDocument {
    fn default() -> Self {
        Self::new()
    }
}

impl HTMLDocument {
    pub fn new() -> Self {
        let mut document = Self {
            nodes: Vec::new(),
            root: NodeID::new(0),
        };
        let root = document.push_node(NodeKind::Document, None);
        document.root = root;
        document
    }

    pub fn push_node(&mut self, kind: NodeKind, span: Option<SourceSpan>) -> NodeID {
        let id = NodeID::new(self.nodes.len());
        self.nodes.push(Node {
            parent: None,
            children: Vec::new(),
            kind,
            span,
        });
        id
    }

    pub fn append_child(&mut self, parent: NodeID, child: NodeID) {
        self.nodes[child.get()].parent = Some(parent);
        self.nodes[parent.get()].children.push(child);
    }

    /// Removes `node` from its parent, if it has one.
    ///
    /// The removed node and its descendants remain in the document arena with
    /// their existing stable IDs. Removing a detached node, the document root,
    /// or an invalid node ID has no effect.
    pub fn remove(&mut self, node: NodeID) {
        let Some(parent) = self.node(node).and_then(|node| node.parent) else {
            return;
        };

        if let Some(children) = self
            .nodes
            .get_mut(parent.get())
            .map(|node| &mut node.children)
        {
            children.retain(|child| *child != node);
        }
        if let Some(removed) = self.nodes.get_mut(node.get()) {
            removed.parent = None;
        }
    }

    pub fn node(&self, id: NodeID) -> Option<&Node> {
        self.nodes.get(id.get())
    }

    /// Returns a live `DOMTokenList` for an element's `class` attribute.
    ///
    /// The handle mutates the associated element through its `add`, `remove`,
    /// `toggle`, `replace`, and `value` operations. The document is borrowed
    /// mutably for the lifetime of the handle, matching the exclusive access
    /// required for DOM mutations in this Rust model.
    pub fn class_list(&mut self, element: NodeID) -> Result<DOMTokenList<'_>, DomError> {
        if !matches!(
            self.node(element).map(|node| &node.kind),
            Some(NodeKind::Element(_))
        ) {
            return Err(DomError::NotFound);
        }
        Ok(DOMTokenList::new(self, element))
    }

    pub(crate) fn node_mut(&mut self, id: NodeID) -> Option<&mut Node> {
        self.nodes.get_mut(id.get())
    }

    /// Creates a detached HTML element using the DOM `createElement` rules.
    ///
    /// ASCII uppercase characters are lowercased because this arena represents
    /// an HTML document. The returned node has no parent, no attributes, and
    /// the HTML namespace. Invalid names return [`DomError::InvalidCharacter`].
    /// The node is owned by this arena; use the returned index with [`Self::node`].
    /// XML documents and custom-element creation options are not supported.
    pub fn create_element(&mut self, local_name: &str) -> Result<NodeID, DomError> {
        if !is_valid_element_local_name(local_name) {
            return Err(DomError::InvalidCharacter);
        }

        let element = ElementData {
            name: local_name.to_ascii_lowercase(),
            namespace: Namespace::Html,
            attributes: Vec::new(),
        };
        Ok(self.push_node(NodeKind::Element(element), None))
    }

    /// Sets an element's attribute using the DOM `setAttribute` rules.
    ///
    /// HTML attribute names are normalized to ASCII lowercase. Existing
    /// attributes are updated in place so their document order is preserved;
    /// new attributes are appended. Names that are not valid attribute local
    /// names return [`DomError::InvalidCharacter`].
    pub fn set_attribute(
        &mut self,
        element: NodeId,
        qualified_name: &str,
        value: &str,
    ) -> Result<(), DomError> {
        if !is_valid_attribute_local_name(qualified_name) {
            return Err(DomError::InvalidCharacter);
        }

        let node = self.node_mut(element).ok_or(DomError::NotFound)?;
        let NodeKind::Element(element_data) = &mut node.kind else {
            return Err(DomError::NotFound);
        };

        let name = match element_data.namespace {
            Namespace::Html => qualified_name.to_ascii_lowercase(),
            Namespace::Svg | Namespace::MathMl => qualified_name.to_owned(),
        };

        if let Some(attribute) = element_data
            .attributes
            .iter_mut()
            .find(|attribute| attribute.name == name)
        {
            attribute.value = value.to_owned();
        } else {
            element_data.attributes.push(super::attribute::Attribute {
                name,
                value: value.to_owned(),
            });
        }

        Ok(())
    }

    /// Returns the DOM `textContent` value for a node.
    ///
    /// Element nodes return the concatenation of descendant text nodes in tree
    /// order. Text and comment nodes return their own data. Document and
    /// doctype nodes return `None`, matching the DOM Standard; invalid node IDs
    /// also return `None`. This is a read-only snapshot and does not mutate the
    /// document.
    pub fn text_content(&self, id: NodeID) -> Option<String> {
        fn descendant_text(document: &HTMLDocument, id: NodeID, output: &mut String) {
            let Some(node) = document.node(id) else {
                return;
            };
            if let NodeKind::Text(text) = &node.kind {
                output.push_str(text);
            }
            for child in &node.children {
                descendant_text(document, *child, output);
            }
        }

        let node = self.node(id)?;
        match &node.kind {
            NodeKind::Element(_) => {
                let mut output = String::new();
                for child in &node.children {
                    descendant_text(self, *child, &mut output);
                }
                Some(output)
            }
            NodeKind::Text(text) | NodeKind::Comment(text) => Some(text.clone()),
            NodeKind::Document | NodeKind::Doctype { .. } => None,
            NodeKind::None => None,
        }
    }

    /// Serializes the document's child nodes as an HTML fragment.
    ///
    /// This follows the HTML fragment serialization rules for the node kinds
    /// represented by this arena. It escapes text and attribute data, emits
    /// comments and doctypes, and omits end tags for HTML void elements. The
    /// arena does not model template contents, shadow roots, or all namespace-
    /// specific serialization details, so those parts of the HTML Standard
    /// are not implemented.
    pub fn inner_html(&self) -> String {
        let mut output = String::new();
        if let Some(root) = self.node(self.root) {
            for child in &root.children {
                self.serialize_node(*child, &mut output, false);
            }
        }
        output
    }

    fn serialize_node(&self, id: NodeId, output: &mut String, raw_text: bool) {
        let Some(node) = self.node(id) else {
            return;
        };
        match &node.kind {
            NodeKind::Document => {
                for child in &node.children {
                    self.serialize_node(*child, output, false);
                }
            }
            NodeKind::Doctype {
                name,
                public_id,
                system_id,
            } => {
                output.push_str("<!DOCTYPE");
                if let Some(name) = name {
                    output.push(' ');
                    output.push_str(name);
                }
                if let Some(public_id) = public_id {
                    output.push_str(" PUBLIC \"");
                    output.push_str(&escape_attribute(public_id));
                    output.push('"');
                    if let Some(system_id) = system_id {
                        output.push_str(" \"");
                        output.push_str(&escape_attribute(system_id));
                        output.push('"');
                    }
                } else if let Some(system_id) = system_id {
                    output.push_str(" SYSTEM \"");
                    output.push_str(&escape_attribute(system_id));
                    output.push('"');
                }
                output.push('>');
            }
            NodeKind::Element(element) => {
                output.push('<');
                output.push_str(&element.name);
                for attribute in &element.attributes {
                    output.push(' ');
                    output.push_str(&attribute.name);
                    output.push_str("=\"");
                    output.push_str(&escape_attribute(&attribute.value));
                    output.push('"');
                }
                output.push('>');
                let is_void =
                    element.namespace == Namespace::Html && is_void_html_element(&element.name);
                if !is_void {
                    let child_raw_text = element.namespace == Namespace::Html
                        && matches!(element.name.as_str(), "script" | "style");
                    for child in &node.children {
                        self.serialize_node(*child, output, child_raw_text);
                    }
                    output.push_str("</");
                    output.push_str(&element.name);
                    output.push('>');
                }
            }
            NodeKind::Text(text) => {
                if raw_text {
                    output.push_str(text);
                } else {
                    output.push_str(&escape_text(text));
                }
            }
            NodeKind::Comment(comment) => {
                output.push_str("<!--");
                output.push_str(comment);
                output.push_str("-->");
            }
            NodeKind::None => {}
        }
    }

    /// Returns the source text of inline `script` elements in tree order.
    ///
    /// External scripts are not fetched yet, so only the text contained by
    /// each `script` element is returned. The returned strings are snapshots;
    /// evaluating them does not mutate this document.
    pub fn script_sources(&self) -> Vec<String> {
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                let NodeKind::Element(element) = &node.kind else {
                    return None;
                };
                (element.name == "script")
                    .then(|| self.text_content(NodeID::new(index)).unwrap_or_default())
            })
            .collect()
    }

    pub fn get_element_by_id(&self, value: &str) -> Option<NodeID> {
        fn find_in_tree(document: &HTMLDocument, parent: NodeID, value: &str) -> Option<NodeID> {
            let children = document.node(parent)?.children.clone();

            for child in children {
                let node = document.node(child)?;
                if let NodeKind::Element(element) = &node.kind
                    && element
                        .attributes
                        .iter()
                        .any(|attribute| attribute.name == "id" && attribute.value == value)
                {
                    return Some(child);
                }

                if let Some(found) = find_in_tree(document, child, value) {
                    return Some(found);
                }
            }

            None
        }

        // The DOM algorithm searches descendants in tree order. The document
        // node itself is not an element and therefore is never a candidate.
        find_in_tree(self, self.root, value)
    }
}

fn escape_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('\u{00a0}', "&nbsp;")
        .replace('<', "&lt;")
}

fn escape_attribute(value: &str) -> String {
    escape_text(value).replace('"', "&quot;")
}

fn is_void_html_element(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn is_ascii_whitespace(character: char) -> bool {
    matches!(character, '\t' | '\n' | '\x0c' | '\r' | ' ')
}

fn is_ascii_alpha(character: char) -> bool {
    character.is_ascii_alphabetic()
}

fn is_valid_element_local_name(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };

    if is_ascii_alpha(first) {
        return characters.all(|character| {
            !is_ascii_whitespace(character)
                && character != '\0'
                && character != '/'
                && character != '>'
        });
    }

    if first != ':' && first != '_' && (first as u32) < 0x80 {
        return false;
    }

    characters.all(|character| {
        character.is_ascii_alphanumeric()
            || matches!(character, '-' | '.' | ':' | '_')
            || (character as u32) >= 0x80
    })
}

fn is_valid_attribute_local_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|character| {
            !is_ascii_whitespace(character)
                && character != '\0'
                && character != '/'
                && character != '='
                && character != '>'
        })
}
