use std::collections::HashMap;

pub type ElementID = u32;

pub enum Element{
    Document(Document),
    HTMLElement(HTMLElement),
    TextElement(TextElement),
}

pub struct HTMLElement{
    name : String,
    element_id : ElementID,
    attributes : HashMap<String, String>,
    child_ids : Vec<ElementID>,
}

pub struct TextElement{
    text : String,
    element_id : ElementID,
}

pub struct Document{
    doc_type : String,
    elements : HashMap<ElementID, Element>,
    child_ids : Vec<ElementID>,
}