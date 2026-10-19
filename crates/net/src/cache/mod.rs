//! Process-local response cache implementation.

mod memory;

pub(crate) use memory::{ResponseCache, StoredResponse};
