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

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.0)
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
