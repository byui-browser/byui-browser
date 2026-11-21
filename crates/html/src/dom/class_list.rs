use super::{Attribute, DomError};

/// A live view of an element's whitespace-separated `class` attribute classes.
///
/// This follows the DOM Standard's `ClassList` algorithms. The list is
/// associated with one element and remains synchronized with that element's
/// `class` attribute while the handle is borrowed.
pub struct ClassList<'attribute> {
    class_attribute: &'attribute mut Attribute,
    stored_classes_string: String,
    stored_classes: Vec<String>,
}

impl<'attribute> ClassList<'attribute> {
    pub(crate) fn new(class_attribute: &'attribute mut Attribute) -> Self {
        Self {
            class_attribute,
            stored_classes_string: String::new(),
            stored_classes: Vec::new(),
        }
    }

    /// Updates stored classes to match class attribute.
    fn sync_classes(&mut self) {
        let classes_string = self.class_attribute.get_value();
        if classes_string != &self.stored_classes_string {
            self.stored_classes_string = classes_string.clone();
            self.stored_classes = parse_classes(classes_string);
        }
    }

    /// Returns a snapshot of the classes in document order.
    pub fn get_classes(&mut self) -> Vec<String> {
        self.sync_classes();
        self.stored_classes.clone()
    }

    /// Returns the number of classes.
    pub fn length(&mut self) -> usize {
        self.sync_classes();
        self.stored_classes.len()
    }

    /// Returns the class at `index`, or `None` when the index is out of range.
    pub fn item(&mut self, index: usize) -> Option<String> {
        self.sync_classes();
        self.stored_classes.get(index).cloned()
    }

    /// Returns whether `class` is present.
    pub fn contains(&mut self, class: &str) -> bool {
        self.sync_classes();
        self.stored_classes.iter().any(|current| current == class)
    }

    /// Returns the serialized `class` attribute value.
    pub fn value(&self) -> String {
        self.class_attribute.get_value().clone()
    }

    /// Sets the raw `class` attribute value to `value`.
    pub fn set_value(&mut self, value: &str) {
        self.class_attribute.set_value(value.to_owned());
    }

    /// Adds each class that is not already present.
    pub fn add(&mut self, classes: &[&str]) -> Result<(), DomError> {
        validate_classes(classes)?;
        self.sync_classes();

        let mut current = self.stored_classes.clone();
        let original_len = current.len();
        for class in classes {
            if !current.iter().any(|existing| existing.as_str() == *class) {
                log::trace!("classList add {class:?} on node");
                current.push((*class).to_owned());
            }
        }

        if current.len() != original_len {
            self.write_classes(current);
        }
        Ok(())
    }

    /// Removes each class that is present.
    pub fn remove(&mut self, classes: &[&str]) -> Result<(), DomError> {
        validate_classes(classes)?;
        self.sync_classes();

        let current = self
            .stored_classes
            .iter()
            .filter(|current| {
                let removed = classes.contains(&current.as_str());
                if removed {
                    log::trace!("classList remove {current:?} on node");
                }
                !removed
            })
            .cloned()
            .collect::<Vec<_>>();

        if current.len() != self.stored_classes.len() {
            self.write_classes(current);
        }
        Ok(())
    }

    /// Adds or removes `class`, honoring `force` when it is provided.
    pub fn toggle(&mut self, class: &str, force: Option<bool>) -> Result<bool, DomError> {
        validate_classes(&[class])?;
        self.sync_classes();

        let present = self.stored_classes.iter().any(|current| current == class);
        let should_have = force.unwrap_or(!present);
        if should_have && !present {
            self.add(&[class])?;
        } else if !should_have && present {
            self.remove(&[class])?;
        }
        Ok(should_have)
    }

    /// Replaces `class` with `new_class` when `class` is present.
    pub fn replace(&mut self, class: &str, new_class: &str) -> Result<bool, DomError> {
        validate_classes(&[class, new_class])?;
        self.sync_classes();

        let Some(index) = self
            .stored_classes
            .iter()
            .position(|current| current == class)
        else {
            return Ok(false);
        };
        let mut current = self.stored_classes.clone();
        log::trace!("classList replace {class:?} with {new_class:?} on node");
        current.remove(index);
        if !current.iter().any(|current| current.as_str() == new_class) {
            current.insert(index, new_class.to_owned());
        }
        self.write_classes(current);
        Ok(true)
    }

    /// Sets class attribute's value with class vector as string.
    fn write_classes(&mut self, classes: Vec<String>) {
        let value = classes.join(" ");
        self.class_attribute.set_value(value.clone());
        self.stored_classes_string = value;
        self.stored_classes = classes;
    }
}

/// Converts string of classes into a vector.
fn parse_classes(classes_string: &str) -> Vec<String> {
    let mut classes: Vec<String> = Vec::new();
    for class in classes_string.split(is_ascii_whitespace) {
        if !class.is_empty() && !classes.iter().any(|current| current.as_str() == class) {
            classes.push(class.to_owned());
        }
    }
    classes
}

/// Checks if all class names in the 'classes' list are valid.
fn validate_classes(classes: &[&str]) -> Result<(), DomError> {
    for class in classes {
        if class.is_empty() {
            log::debug!("classList rejected empty class");
            return Err(DomError::Syntax);
        }
        if class.chars().any(is_ascii_whitespace) {
            log::debug!("classList rejected class {class:?}: contains ASCII whitespace");
            return Err(DomError::InvalidClass);
        }
    }
    Ok(())
}

// Checks if character is whitespace.
fn is_ascii_whitespace(character: char) -> bool {
    matches!(character, '\t' | '\n' | '\x0c' | '\r' | ' ')
}
