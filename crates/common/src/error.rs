//! Shared error types for cross-crate and cross-process failures.

use std::fmt;

/// Workspace-wide error type for failures that cross a crate boundary.
///
/// Crate-internal errors should stay crate-internal; convert to this only
/// at the public interface.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Add variants
// as real cross-crate failures appear; changes require cross-team review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserError {
    /// Input could not be parsed by the named crate.
    Parse {
        /// Crate that produced the error (e.g. `"html"`).
        source_crate: &'static str,
        /// Human-readable description.
        message: String,
    },
    /// The requested feature has not been implemented yet.
    Unimplemented(&'static str),
}

impl fmt::Display for BrowserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse {
                source_crate,
                message,
            } => write!(f, "{source_crate}: parse error: {message}"),
            Self::Unimplemented(what) => write!(f, "not implemented: {what}"),
        }
    }
}

impl std::error::Error for BrowserError {}

/// Convenience alias for results that cross a crate boundary.
pub type Result<T> = std::result::Result<T, BrowserError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_error_display_names_crate() {
        let err = BrowserError::Parse {
            source_crate: "html",
            message: "unexpected EOF".into(),
        };
        assert_eq!(err.to_string(), "html: parse error: unexpected EOF");
    }

    #[test]
    fn unimplemented_display() {
        assert_eq!(
            BrowserError::Unimplemented("css cascade").to_string(),
            "not implemented: css cascade"
        );
    }
}
