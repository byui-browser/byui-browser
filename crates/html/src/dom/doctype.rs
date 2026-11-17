
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Doctype {
    name: Option<String>,
    public_id: Option<String>,
    system_id: Option<String>,
}

impl Doctype {
    /// Constructs Doctype and sets immutable values.
    pub fn new(name : Option<String>, public_id : Option<String>, system_id : Option<String>) -> Self {
        Self {
            name,
            public_id,
            system_id,
        }
    }

    /// Gets doctype name.
    pub const fn get_name(&self) -> &Option<String> {
        &self.name
    }

    /// Gets public id.
    pub const fn get_public_id(&self) -> &Option<String> {
        &self.public_id
    }

    /// Gets system id.
    pub const fn get_system_id(&self) -> &Option<String> {
        &self.system_id
    }
}