//! Policy and filesystem primitives for browser-authorized `file:` access.
//!
//! This module deliberately does not depend on the `net` crate. The browser
//! layer adapts these primitives to `net::FileProvider` when composing a
//! request controller.

mod error;
mod filesystem;
mod policy;

pub use error::FileAccessError;
pub use filesystem::{FileResource, FileSystem, LocalFileSystem, OpenedFile, SharedFileSystem};
pub use policy::{AuthorizedPath, FileAccessPolicy};
