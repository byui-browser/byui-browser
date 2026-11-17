//! Fetch-compatible HTTP header collection and filtering.

mod collection;
mod cors;
mod guard;
mod validation;

pub use collection::Headers;
pub(crate) use cors::{cors_unsafe_request_header_names, is_cors_safelisted_header};
pub use guard::HeaderGuard;

#[cfg(test)]
mod tests;
