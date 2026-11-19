//! Process-local HTTP response cache implementation.
//!
//! This module owns transport-independent cache matching and response capture.
//! Browser and storage layers supply partition identity and may later provide a
//! durable backend; they do not need to reimplement Fetch cache algorithms.
//! The current in-memory backend intentionally remains a documented subset
//! until freshness, revalidation, and invalidation work is completed.

mod memory;
mod preflight;

pub(crate) use memory::{CacheLookup, ResponseCache, StoredResponse};
pub(crate) use preflight::{PreflightCache, PreflightPermissions};
