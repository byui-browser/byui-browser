use common::ids::NodeId as CommonNodeId;
use std::collections::BTreeMap;

/// A stable index into an [`HtmlDocument`] arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub usize);

impl NodeId {
    pub const fn index(self) -> usize {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    Doctype(String),
    Comment(String),
    StartTag(String),
    EndTag(String),
    Text(String),
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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum QuirksMode {
    #[default]
    NoQuirks,
    LimitedQuirks,
    Quirks,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlDocument {
    pub nodes: Vec<Node>,
    pub root: NodeId,
    pub quirks_mode: QuirksMode,
}

impl Default for HtmlDocument {
    fn default() -> Self {
        Self::new()
    }
}

impl HtmlDocument {
    pub fn new() -> Self {
        let mut document = Self {
            nodes: Vec::new(),
            root: NodeId(0),
            quirks_mode: QuirksMode::NoQuirks,
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
        fn collect(document: &HtmlDocument, id: NodeId, output: &mut String) {
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
        self.nodes.iter().enumerate().find_map(|(index, node)| {
            let NodeKind::Element(element) = &node.kind else {
                return None;
            };
            element
                .attributes
                .iter()
                .find(|attribute| attribute.name == "id" && attribute.value == value)
                .map(|_| NodeId(index))
        })
    }
}

pub type HTMLDocument = HtmlDocument;

/// Compatibility projection returned by the original selector API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HTMLElement {
    pub id: CommonNodeId,
    pub name: String,
    pub attributes: BTreeMap<String, String>,
    pub parent: Option<CommonNodeId>,
    pub children: Vec<CommonNodeId>,
    pub text: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Dom {
    pub nodes: Vec<LegacyNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyNode {
    pub id: CommonNodeId,
    pub name: String,
    pub parent: Option<CommonNodeId>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub url: String,
}
