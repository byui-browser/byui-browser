use std::fmt;

/// Error returned when a DOM operation receives an invalid element name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomError {
    /// The supplied name is not a valid element local name.
    InvalidCharacter,
}

impl fmt::Display for DomError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCharacter => formatter.write_str("InvalidCharacterError"),
        }
    }
}

impl std::error::Error for DomError {}

/// A stable index into an [`HTMLDocument`] arena.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub usize);

impl NodeId {
    pub const fn index(self) -> usize {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub kind: NodeKind,
    pub span: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    Document,
    Doctype {
        name: Option<String>,
        public_id: Option<String>,
        system_id: Option<String>,
    },
    Element(ElementData),
    Text(String),
    Comment(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementData {
    pub name: String,
    pub namespace: Namespace,
    pub attributes: Vec<Attribute>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Namespace {
    Html,
    Svg,
    MathMl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceSpan {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HTMLDocument {
    pub nodes: Vec<Node>,
    pub root: NodeId,
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
            root: NodeId(0),
        };
        let root = document.push_node(NodeKind::Document, None);
        document.root = root;
        document
    }

    pub fn push_node(&mut self, kind: NodeKind, span: Option<SourceSpan>) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(Node {
            parent: None,
            children: Vec::new(),
            kind,
            span,
        });
        id
    }

    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        self.nodes[child.0].parent = Some(parent);
        self.nodes[parent.0].children.push(child);
    }

    /// Removes `node` from its parent, if it has one.
    ///
    /// The removed node and its descendants remain in the document arena with
    /// their existing stable IDs. Removing a detached node, the document root,
    /// or an invalid node ID has no effect.
    pub fn remove(&mut self, node: NodeId) {
        let Some(parent) = self.node(node).and_then(|node| node.parent) else {
            return;
        };

        if let Some(children) = self.nodes.get_mut(parent.0).map(|node| &mut node.children) {
            children.retain(|child| *child != node);
        }
        if let Some(removed) = self.nodes.get_mut(node.0) {
            removed.parent = None;
        }
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.0)
    }

    /// Creates a detached HTML element using the DOM `createElement` rules.
    ///
    /// ASCII uppercase characters are lowercased because this arena represents
    /// an HTML document. The returned node has no parent, no attributes, and
    /// the HTML namespace. Invalid names return [`DomError::InvalidCharacter`].
    /// The node is owned by this arena; use the returned index with [`Self::node`].
    /// XML documents and custom-element creation options are not supported.
    pub fn create_element(&mut self, local_name: &str) -> Result<NodeId, DomError> {
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

    pub fn text_content(&self, id: NodeId) -> String {
        fn collect(document: &HTMLDocument, id: NodeId, output: &mut String) {
            let Some(node) = document.node(id) else {
                return;
            };
            if let NodeKind::Text(text) = &node.kind {
                output.push_str(text);
            }
            for child in &node.children {
                collect(document, *child, output);
            }
        }
        let mut output = String::new();
        collect(self, id, &mut output);
        output
    }

    pub fn get_element_by_id(&self, value: &str) -> Option<NodeId> {
        fn find_in_tree(document: &HTMLDocument, parent: NodeId, value: &str) -> Option<NodeId> {
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
