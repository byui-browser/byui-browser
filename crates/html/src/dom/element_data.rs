use super::{attribute::Attribute, namespace::Namespace};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementData {
    name: String,
    namespace: Namespace,
    attributes: Vec<Attribute>,
}

impl ElementData {
    // Constructs ElementData.
    pub fn new(name: String, namespace: Namespace, attributes: Vec<Attribute>) -> Self {
        Self {
            name,
            namespace,
            attributes,
        }
    }

    // Gets reference to element name.
    pub const fn get_name(&self) -> &String {
        &self.name
    }
    // Sets element name.
    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }

    // Gets reference to namespace.
    pub const fn get_namespace(&self) -> &Namespace {
        &self.namespace
    }
    // Sets element namespace.
    pub fn set_namespace(&mut self, namespace: Namespace) {
        self.namespace = namespace;
    }

    // Gets reference to element attribute vector.
    pub const fn get_attributes(&self) -> &Vec<Attribute> {
        &self.attributes
    }
    // Sets element attribute vector.
    pub fn set_attributes(&mut self, attributes: Vec<Attribute>) {
        self.attributes = attributes;
    }

    // Gets attribute with name.
    pub fn get_attribute(&self, name: String) -> Option<&Attribute> {
        if let Some(index) = self
            .attributes
            .iter()
            .position(|attribute_ref| attribute_ref.get_name() == &name)
        {
            return Option::Some(&self.attributes[index]);
        } else {
            return Option::None;
        }
    }
    // Updates or adds element attribute.
    pub fn set_attribute(&mut self, attribute: Attribute) {
        if let Some(index) = self
            .attributes
            .iter()
            .position(|attribute_ref| attribute_ref.get_name() == attribute.get_name())
        {
            // Updates existing attribute.
            self.attributes[index].set_value(String::from(attribute.get_value()));
        } else {
            // Adds new attribute.
            self.add_attribute(attribute);
        }
    }
    // Adds element attribute.
    fn add_attribute(&mut self, attribute: Attribute) {
        self.attributes.push(attribute);
    }

    // Removes element attribute.
    pub fn remove_attribute(&mut self, name: String) {
        if let Some(index) = self
            .attributes
            .iter()
            .position(|attribute| *attribute.get_name() == name)
        {
            self.attributes.remove(index);
        }
    }
}
