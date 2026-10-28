use super::{attribute::Attribute, namespace::Namespace};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementData {
    pub name: String,
    pub namespace: Namespace,
    pub attributes: Vec<Attribute>,
}

impl ElementData {
    pub const fn get_name(&self) -> &String {
        &self.name
    }
    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }

    pub const fn get_namespace(&self) -> &Namespace {
        &self.namespace
    }
    pub fn set_namespace(&mut self, namespace: Namespace) {
        self.namespace = namespace;
    }

    pub const fn get_attributes(&self) -> &Vec<Attribute> {
        &self.attributes
    }
    pub fn set_attributes(&mut self, attributes: Vec<Attribute>) {
        self.attributes = attributes;
    }

    pub fn set_attribute(&mut self, attribute: Attribute) {
        if let Some(index) = self
            .attributes
            .iter()
            .position(|attribute_ref| attribute_ref.get_name() == attribute.get_name())
        {
            self.attributes[index].set_value(String::from(attribute.get_value()));
        } else {
            self.add_attribute(attribute);
        }
    }
    fn add_attribute(&mut self, attribute: Attribute) {
        self.attributes.push(attribute);
    }

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
