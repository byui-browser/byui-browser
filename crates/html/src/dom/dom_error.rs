use std::fmt;

/// Error returned when a DOM operation receives an invalid element name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomError {
    /// The supplied name is not a valid element local name.
    InvalidCharacter,
}

impl fmt::Display for DomError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCharacter => formatter.write_str("InvalidCharacterError"),
        }
    }
}

impl std::error::Error for DomError {}