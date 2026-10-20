//! Policy and filesystem primitives for browser-authorized `file:` access.
//!
//! This module deliberately does not depend on the `net` crate. The browser
//! layer adapts these primitives to `net::FileProvider` when composing a
//! request controller.

mod error;
mod filesystem;
mod policy;
mod request;

pub use error::FileAccessError;
pub use filesystem::{CapFileSystem, FileResource, FileSystem, OpenedFile, SharedFileSystem};
pub use policy::{AuthorizedPath, FileAccessPolicy};
pub use request::{
    FileAccessPermission, FileOrigin, FileRequestContext, FileRequestKind, FileSandboxId,
};
