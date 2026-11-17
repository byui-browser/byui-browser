use super::{
    attribute::Attribute, doctype::Doctype, dom_error::DomError, dom_token_list::DOMTokenList, 
    element_data::ElementData, namespace::Namespace, node::Node, node_id::NodeID, 
    node_kind::NodeKind,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HTMLDocument {
    nodes: Vec<Node>,
    root: NodeID,
}

impl Default for HTMLDocument {
    fn default() -> Self {
        Self::new()
    }
}

impl HTMLDocument {
    /// Constructs HTMLDocument.
    pub fn new() -> Self {
        let mut document = Self {
            nodes: Vec::new(),
            root: NodeID::new(0),
        };
        let root = document.push_node(NodeKind::Document);
        document.root = root;
        document
    }

    // Gets root node of HTMLDocument.
    pub fn get_root(&self) -> &NodeID {
        &self.root
    }

    // Gets nodes list.
    pub fn get_nodes(&self) -> &Vec<Node> {
        &self.nodes
    }

    /// Gets nodes list length.
    pub fn get_nodes_count(&self) -> usize {
        self.nodes.len()
    }

    /// Gets node from its id.
    /// 
    /// Returns None if not found.
    pub fn get_node(&self, id: NodeID) -> Option<&Node> {
        self.nodes.get(id.get())
    }

    /// Gets mutable node from its id.
    /// 
    /// Returns None if not found.
    pub(crate) fn get_mut_node(&mut self, id: NodeID) -> Option<&mut Node> {
        self.nodes.get_mut(id.get())
    }


    /// Adds new orphan node to HTMLDocument and returns its id.
    pub fn push_node(&mut self, kind: NodeKind) -> NodeID {
        let id = NodeID::new(self.nodes.len());
        let node = Node::new(None, Vec::new(), kind);
        self.nodes.push(node);
        id
    }

    /// Parents node to another node.
    pub fn append_child(&mut self, parent: NodeID, child: NodeID) {

        // Removes child from previous parent.
        if let Some(prev_parent) = self.get_node(child).and_then(Node::get_parent){
            if let Some(prev_parent_node) = self.get_mut_node(prev_parent) {
                prev_parent_node.remove_child(child);
            }
        }

        // Adds child to new parent.
        if let Some(parent_node) = self.get_mut_node(parent) {
            parent_node.add_child(child);
        }

        // Sets new parent of child.
        if let Some(child_node) = self.get_mut_node(child) {
            child_node.set_parent(Some(parent));
        }

        log::trace!("append node {} to parent {}", child.get(), parent.get());
    }

    /// Removes `node` from its parent, if it has one.
    ///
    /// The removed node and its descendants remain in the document arena with
    /// their existing stable IDs. Removing a detached node, the document root,
    /// or an invalid node ID has no effect.
    pub fn remove(&mut self, node: NodeID) {
        let Some(parent) = self.get_node(node).and_then(Node::get_parent) else {
            return;
        };

        if let Some(parent_node) = self.get_mut_node(parent) {
            parent_node.remove_child(node);
        }

        // Sets node to be removed parent to None.
        if let Some(removed) = self.nodes.get_mut(node.get()) {
            removed.set_parent(None);
        }
    }

    /// Returns a live `DOMTokenList` for an element's `class` attribute.
    ///
    /// The handle mutates the associated element through its `add`, `remove`,
    /// `toggle`, `replace`, and `value` operations. The document is borrowed
    /// mutably for the lifetime of the handle, matching the exclusive access
    /// required for DOM mutations in this Rust model.
    pub fn class_list(&mut self, element: NodeID) -> Result<DOMTokenList<'_>, DomError> {
        if !matches!(
            self.get_node(element).map(|node| node.get_kind()),
            Some(NodeKind::Element(_))
        ) {
            log::debug!(
                "class_list failed: node {} is not an element",
                element.get()
            );
            return Err(DomError::NotFound);
        }
        Ok(DOMTokenList::new(self, element))
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
            log::debug!("create_element rejected invalid tag name {local_name:?}");
            return Err(DomError::InvalidCharacter);
        }

        let element =
            ElementData::new(local_name.to_ascii_lowercase(), Namespace::Html, Vec::new());
        let id = self.push_node(NodeKind::Element(element));
        log::trace!("created element <{local_name}> as node {}", id.get());
        Ok(id)
    }

    /// Sets an element's attribute using the DOM `setAttribute` rules.
    ///
    /// HTML attribute names are normalized to ASCII lowercase. Existing
    /// attributes are updated in place so their document order is preserved;
    /// new attributes are appended. Names that are not valid attribute local
    /// names return [`DomError::InvalidCharacter`].
    pub fn set_attribute(
        &mut self,
        element: NodeID,
        qualified_name: &str,
        value: &str,
    ) -> Result<(), DomError> {

        // Checks if attriute name is valid.
        if !is_valid_attribute_local_name(qualified_name) {
            log::debug!(
                "set_attribute on node {} rejected invalid attribute name {qualified_name:?}",
                element.get()
            );
            return Err(DomError::InvalidCharacter);
        }

        if let Some(node) = self.get_mut_node(element) {
            let node_kind = node.get_mut_kind();
            let mut element_data = match node_kind {
                NodeKind::Element(element_data) => element_data,
                _ => return Err(DomError::NotFound),
            };

            // Corrects name casing.
            let name = match element_data.get_namespace() {
                Namespace::Html => qualified_name.to_ascii_lowercase(),
                Namespace::Svg | Namespace::MathMl => qualified_name.to_owned(),
            };

            // Sets and logs attribute
            log::trace!("set attribute {name:?} on node {}", element.get());
            element_data.set_attribute(Attribute::new(name, String::from(value)));

        } else {
            return Err(DomError::NotFound);
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
        // Function recursively gets all the text content from a node's descendants.
        fn descendant_text(document: &HTMLDocument, id: NodeID, output: &mut String) {
            // Gets valid node.
            let Some(node) = document.get_node(id) else {
                return;
            };
            match node.get_kind() {
                // Repeats operation for children.
                NodeKind::Element(_) => {
                    for child in node.get_children() {
                        descendant_text(document, *child, output);
                    }
                },
                // Adds text from text element to output.
                NodeKind::Text(text) => { output.push_str(text); },
                // Otherwise just return.
                _ => return
            }
        }

        let node = self.get_node(id)?;
        match node.get_kind() {
            // Gets all text content from child nodes.
            NodeKind::Element(_) => {
                let mut output = String::new();
                for child in node.get_children() {
                    descendant_text(self, *child, &mut output);
                }
                Some(output)
            }
            // Returns text content of text and comment element.
            NodeKind::Text(text) | NodeKind::Comment(text) => Some(text.clone()),
            // Otherwise return none
            _ => None
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
        if let Some(root) = self.get_node(self.root) {
            for child in root.get_children() {
                self.serialize_node(*child, &mut output, false);
            }
        }
        output
    }

    /// Serializes the node `id` and its descendants as an HTML fragment, or
    /// returns `None` when `id` is not a node of this document.
    ///
    /// This uses the same rules and limitations as [`Self::inner_html`]; the
    /// node itself is included. Names are written as the parser stored them,
    /// which is ASCII lowercase, also for SVG content.
    pub fn outer_html(&self, id: NodeID) -> Option<String> {
        self.node(id)?;
        let mut output = String::new();
        self.serialize_node(id, &mut output, false);
        Some(output)
    }

    fn serialize_node(&self, id: NodeID, output: &mut String, raw_text: bool) {
        let Some(node) = self.get_node(id) else {
            return;
        };
        match node.get_kind() {
            NodeKind::Document => {
                for child in node.get_children() {
                    self.serialize_node(*child, output, false);
                }
            }
            NodeKind::Doctype(doctype) => { 
                self.serialize_doctype(output, doctype);
            },
            NodeKind::Element(element) => {
                self.serialize_element(output, element, node);
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

    /// Serializes doctype and appends it to output string.
    fn serialize_doctype(&self, output: &mut String, doctype : &Doctype ) {
        output.push_str("<!DOCTYPE");
        if let Some(name) = doctype.get_name() {
            output.push(' ');
            output.push_str(name);
        }
        if let Some(public_id) = doctype.get_public_id() {
            output.push_str(" PUBLIC \"");
            output.push_str(&escape_attribute(public_id));
            output.push('"');
            if let Some(system_id) = doctype.get_system_id() {
                output.push_str(" \"");
                output.push_str(&escape_attribute(system_id));
                output.push('"');
            }
        } else if let Some(system_id) = doctype.get_system_id() {
            output.push_str(" SYSTEM \"");
            output.push_str(&escape_attribute(system_id));
            output.push('"');
        }
        output.push('>');
    }

    /// Serializes element_data and appends it to output string.
    fn serialize_element(&self, output: &mut String, element: &ElementData, node: &Node) {
        output.push('<');
        output.push_str(element.get_name());
        for attribute in element.get_attributes() {
            output.push(' ');
            output.push_str(attribute.get_name());
            output.push_str("=\"");
            output.push_str(&escape_attribute(attribute.get_value()));
            output.push('"');
        }
        output.push('>');
        let is_void = *element.get_namespace() == Namespace::Html
            && is_void_html_element(element.get_name());
        if !is_void {
            let child_raw_text = *element.get_namespace() == Namespace::Html
                && matches!(element.get_name().as_str(), "script" | "style");
            for child in node.get_children() {
                self.serialize_node(*child, output, child_raw_text);
            }
            output.push_str("</");
            output.push_str(element.get_name());
            output.push('>');
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
                let NodeKind::Element(element) = node.get_kind() else {
                    return None;
                };
                (element.get_name() == "script")
                    .then(|| self.text_content(NodeID::new(index)).unwrap_or_default())
            })
            .collect()
    }

    /// Gets first element with HTML #id
    pub fn get_element_by_id(&self, value: &str) -> Option<NodeID> {
        fn find_in_tree(document: &HTMLDocument, parent: NodeID, value: &str) -> Option<NodeID> {
            let children = document.get_node(parent)?.get_children().clone();

            for child in children {
                let node = document.get_node(child)?;
                if let NodeKind::Element(element) = node.get_kind()
                    && element.get_attributes().iter().any(|attribute| {
                        attribute.get_name() == "id" && attribute.get_value() == value
                    })
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
