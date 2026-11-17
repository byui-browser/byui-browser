use super::{doctype::Doctype, element_data::ElementData};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    Document,
    Doctype(Doctype),
    Element(ElementData),
    Text(String),
    Comment(String),
    /// Placeholder node kind for an uninitialized arena slot.
    None,
}
