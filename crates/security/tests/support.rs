//! Reusable fixtures for Security contract tests.

#![allow(dead_code)]

use security::{Origin, RendererSecurityContext, StorageKey, StoragePermission};

/// The default origin used by Security and Storage tests.
pub fn primary_origin() -> Origin {
    Origin::with_port("https", "example.com", 443)
}

/// A separate origin used to verify isolation.
pub fn other_origin() -> Origin {
    Origin::with_port("https", "other.example", 443)
}

/// Builds a storage key for an origin.
pub fn storage_key_for(origin: Origin) -> StorageKey {
    StorageKey::new(origin)
}

/// Builds an allowed renderer context for the default origin.
pub fn allowed_context() -> RendererSecurityContext {
    RendererSecurityContext::new(primary_origin(), StoragePermission::Allowed)
}

/// Builds a denied renderer context for the default origin.
pub fn denied_context() -> RendererSecurityContext {
    RendererSecurityContext::new(primary_origin(), StoragePermission::Denied)
}
