use super::element_data::ElementData;

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
