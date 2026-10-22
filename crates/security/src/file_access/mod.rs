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
pub use filesystem::{FileResource, OpenedFile};
pub use request::{FileAccessGrant, FileOrigin, FileRequestContext, FileRequestKind};

use std::path::PathBuf;

use filesystem::CapFileSystem;
use policy::FileAccessPolicy;

/// Browser-owned capability for reading files below a fixed set of trusted roots.
///
/// Construct this value only at the browser or platform trust boundary, after a
/// user choice or equivalent embedder authorization has determined the roots.
/// The capability creates the policy and directory handles together, so callers
/// cannot accidentally combine authorization for one root set with filesystem
/// handles for another. Web content and request-construction code should receive
/// requests and grants, never this capability or the source paths used to create
/// it.
#[derive(Debug)]
pub struct FileAccessCapability {
    policy: FileAccessPolicy,
    filesystem: CapFileSystem,
}

impl FileAccessCapability {
    /// Creates a local-file capability from roots approved by trusted browser or
    /// platform code.
    ///
    /// Each root is canonicalized and opened as a directory capability while
    /// this value is created. An empty list creates a default-deny capability.
    /// Callers must not pass paths supplied by web content.
    pub fn from_trusted_roots(
        allowed_roots: impl IntoIterator<Item = PathBuf>,
    ) -> Result<Self, FileAccessError> {
        let allowed_roots = allowed_roots.into_iter().collect::<Vec<_>>();
        Ok(Self {
            policy: FileAccessPolicy::new(allowed_roots.clone())?,
            filesystem: CapFileSystem::new(allowed_roots)?,
        })
    }

    /// Issues a grant accepted only by this capability for future file opens.
    ///
    /// The browser must retain the grant in browser-owned state and may revoke
    /// it when the associated permission lifetime ends.
    pub fn issue_grant(&self) -> FileAccessGrant {
        self.policy.issue_grant()
    }

    /// Verifies the request context and policy-issued grant before any path is
    /// normalized or opened.
    ///
    /// Browser adapters use this to fail closed before converting an
    /// unauthorized URL into a filesystem path. A successful result does not
    /// authorize a pathname; callers must still use
    /// [`Self::open_authorized_file`] for the eventual open.
    pub fn authorize_request(
        &self,
        context: &FileRequestContext,
        grant: &FileAccessGrant,
    ) -> Result<(), FileAccessError> {
        self.policy.authorize_request(context, grant)
    }

    /// Authorizes and opens one local file through this capability.
    ///
    /// The returned handle is retained by the caller and must be used for all
    /// subsequent reads; callers must not reopen the pathname.
    pub fn open_authorized_file(
        &self,
        context: &FileRequestContext,
        grant: &FileAccessGrant,
        requested: impl AsRef<std::path::Path>,
    ) -> Result<OpenedFile, FileAccessError> {
        self.policy
            .open_authorized_file(context, grant, requested, &self.filesystem)
    }
}
