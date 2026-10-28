use super::{Attribute, DomError, HTMLDocument, NodeID, NodeKind};

/// A live view of an element's whitespace-separated `class` attribute tokens.
///
/// This follows the DOM Standard's `DOMTokenList` algorithms. The list is
/// associated with one element and remains synchronized with that element's
/// `class` attribute while the handle is borrowed.
pub struct DOMTokenList<'document> {
    document: &'document mut HTMLDocument,
    element: NodeID,
}

impl<'document> DOMTokenList<'document> {
    pub(crate) fn new(document: &'document mut HTMLDocument, element: NodeID) -> Self {
        Self { document, element }
    }

    /// Returns the number of class tokens.
    pub fn length(&self) -> usize {
        self.tokens().len()
    }

    /// Returns the token at `index`, or `None` when the index is out of range.
    pub fn item(&self, index: usize) -> Option<String> {
        self.tokens().get(index).cloned()
    }

    /// Returns all class tokens in document order.
    pub fn tokens(&self) -> Vec<String> {
        let Some(NodeKind::Element(element)) =
            self.document.node(self.element).map(|node| &node.kind)
        else {
            return Vec::new();
        };
        element
            .attributes
            .iter()
            .find(|attribute| attribute.get_name() == "class")
            .map(|attribute| parse_tokens(attribute.get_value()))
            .unwrap_or_default()
    }

    /// Returns the serialized `class` attribute value.
    pub fn value(&self) -> String {
        self.class_attribute_value().unwrap_or_default()
    }

    /// Sets the raw `class` attribute value to `value`.
    pub fn set_value(&mut self, value: &str) -> Result<(), DomError> {
        self.write_attribute_value(value)
    }

    /// Returns whether `token` is present.
    pub fn contains(&self, token: &str) -> bool {
        self.tokens().iter().any(|current| current == token)
    }

    /// Adds each token that is not already present.
    pub fn add(&mut self, tokens: &[&str]) -> Result<(), DomError> {
        validate_tokens(tokens)?;
        let mut current = self.tokens();
        for token in tokens {
            if !current.iter().any(|existing| existing.as_str() == *token) {
                current.push((*token).to_owned());
            }
        }
        self.write_tokens(current)
    }

    /// Removes each token that is present.
    pub fn remove(&mut self, tokens: &[&str]) -> Result<(), DomError> {
        validate_tokens(tokens)?;
        let current = self
            .tokens()
            .into_iter()
            .filter(|current| !tokens.contains(&current.as_str()))
            .collect();
        self.write_tokens(current)
    }

    /// Adds or removes `token`, honoring `force` when it is provided.
    pub fn toggle(&mut self, token: &str, force: Option<bool>) -> Result<bool, DomError> {
        validate_tokens(&[token])?;
        let present = self.contains(token);
        let should_have = force.unwrap_or(!present);
        if should_have && !present {
            self.add(&[token])?;
        } else if !should_have && present {
            self.remove(&[token])?;
        }
        Ok(should_have)
    }

    /// Replaces `token` with `new_token` when `token` is present.
    pub fn replace(&mut self, token: &str, new_token: &str) -> Result<bool, DomError> {
        validate_tokens(&[token, new_token])?;
        let mut current = self.tokens();
        let Some(index) = current.iter().position(|current| current == token) else {
            return Ok(false);
        };
        current.remove(index);
        if !current.iter().any(|current| current.as_str() == new_token) {
            current.insert(index, new_token.to_owned());
        }
        self.write_tokens(current)?;
        Ok(true)
    }

    /// Reports whether the token is supported for this list's associated
    /// attribute. Class attributes define no constrained supported-token set,
    /// so the DOM Standard requires this operation to throw `TypeError`.
    pub fn supports(&self, _token: &str) -> Result<bool, DomError> {
        Err(DomError::Type)
    }

    fn write_tokens(&mut self, tokens: Vec<String>) -> Result<(), DomError> {
        if tokens.is_empty() && self.class_attribute_value().is_none() {
            return Ok(());
        }
        self.write_attribute_value(&tokens.join(" "))
    }

    fn class_attribute_value(&self) -> Option<String> {
        let Some(NodeKind::Element(element)) =
            self.document.node(self.element).map(|node| &node.kind)
        else {
            return None;
        };
        element
            .attributes
            .iter()
            .find(|attribute| attribute.get_name() == "class")
            .map(|attribute| attribute.get_value().clone())
    }

    fn write_attribute_value(&mut self, value: &str) -> Result<(), DomError> {
        let Some(NodeKind::Element(element)) = self
            .document
            .node_mut(self.element)
            .map(|node| &mut node.kind)
        else {
            return Err(DomError::NotFound);
        };
        if let Some(attribute) = element
            .attributes
            .iter_mut()
            .find(|attribute| attribute.get_name() == "class")
        {
            attribute.set_value(value.to_owned());
        } else {
            element
                .attributes
                .push(Attribute::new("class".to_owned(), value.to_owned()));
        }
        Ok(())
    }
}

fn parse_tokens(value: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    for token in value.split(is_ascii_whitespace) {
        if !token.is_empty() && !tokens.iter().any(|current| current.as_str() == token) {
            tokens.push(token.to_owned());
        }
    }
    tokens
}

fn validate_tokens(tokens: &[&str]) -> Result<(), DomError> {
    for token in tokens {
        if token.is_empty() {
            return Err(DomError::Syntax);
        }
        if token.chars().any(is_ascii_whitespace) {
            return Err(DomError::InvalidToken);
        }
    }
    Ok(())
}

fn is_ascii_whitespace(character: char) -> bool {
    matches!(character, '\t' | '\n' | '\x0c' | '\r' | ' ')
}
