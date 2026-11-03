#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    name: String,
    value: String,
}

impl Default for Attribute {
    fn default() -> Self {
        Self::new(String::from(""), String::from(""))
    }
}

impl Attribute {
    pub fn new(name: String, value: String) -> Self {
        Self { name, value }
    }

    pub const fn get_name(&self) -> &String {
        &self.name
    }

    pub const fn get_value(&self) -> &String {
        &self.value
    }
    pub fn set_value(&mut self, value: String) {
        self.value = value;
    }
}
