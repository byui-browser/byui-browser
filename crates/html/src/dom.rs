pub mod html_document;

pub mod node;

pub mod node_id;
pub mod node_kind;
pub mod source_span;

pub mod element_data;

pub mod attribute;
pub mod namespace;

pub mod dom_error;

pub use {
    attribute::Attribute, dom_error::DomError, element_data::ElementData,
    html_document::HTMLDocument, namespace::Namespace, node::Node, node_id::NodeId,
    node_kind::NodeKind, source_span::SourceSpan,
};
