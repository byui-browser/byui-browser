//! Fetch request and response body primitives.

mod blob;
mod construct;
mod consume;
mod form_data;
mod metadata;
mod state;
mod stream;
#[cfg(test)]
mod tests;
mod transport;

pub use blob::Blob;
pub use form_data::{FormData, FormDataEntry};
pub use state::{Body, BodyStatus};

pub(crate) use crate::api::error::RequestError;
pub(crate) use state::{BodyPhase, BodySource, BodyState, BoxedBodyStream};
