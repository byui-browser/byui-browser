use super::{node_id::NodeId, node_kind::NodeKind, source_span::SourceSpan};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub kind: NodeKind,
    pub span: Option<SourceSpan>,
}

impl Node {

    pub fn new() -> Self{
        return Self {
            parent: Option::None,
            children: vec![],
            kind: NodeKind::None,
            span: Option::None
        };
    }

    pub fn append_child(node : Node){

    }

    pub fn remove() {
        
    }
}
