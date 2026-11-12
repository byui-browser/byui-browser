use std::fmt;

/// Error returned when a DOM operation receives invalid input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomError {
    /// The supplied name is not a valid element local name.
    InvalidCharacter,
    /// The supplied token is empty.
    Syntax,
    /// The supplied token contains ASCII whitespace.
    InvalidToken,
    /// The operation is not supported for the requested node or attribute.
    Type,
    /// The supplied node ID does not identify an element.
    NotFound,
}

impl fmt::Display for DomError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCharacter => formatter.write_str("InvalidCharacterError"),
            Self::Syntax => formatter.write_str("SyntaxError"),
            Self::InvalidToken => formatter.write_str("InvalidCharacterError"),
            Self::Type => formatter.write_str("TypeError"),
            Self::NotFound => formatter.write_str("NotFoundError"),
        }
    }
}

impl std::error::Error for DomError {}
