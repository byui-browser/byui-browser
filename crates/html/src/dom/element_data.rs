use super::{attribute::Attribute, namespace::Namespace};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementData {
    pub name: String,
    pub namespace: Namespace,
    pub attributes: Vec<Attribute>,
}
