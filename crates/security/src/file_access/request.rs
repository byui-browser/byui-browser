//! Request-context primitives used to authorize browser `file:` access.

/// Origin information supplied by the browser when it asks to access a local file.
///
/// This is deliberately a security-owned representation. Browser adapters must
/// translate their networking origin type before calling [`FileAccessPolicy`].
/// Tuple origins are serialized in their canonical form and are retained for
/// policy decisions and auditing; this initial policy does not grant
/// subresource access for any origin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileOrigin {
    /// The request has no initiating client origin.
    Absent,
    /// The request has an opaque initiating origin.
    Opaque,
    /// A canonical tuple origin, such as `https://example.test`.
    Tuple(String),
    /// The requested resource is a local file.
    LocalFile,
}

/// The browser operation that caused a `file:` request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileRequestKind {
    /// A top-level navigation to a local file.
    Navigation,
    /// A resource requested by another document, worker, or script.
    Subresource,
}

/// Permission state for one attempted local-file access.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileAccessPermission {
    /// The browser has not granted local-file access.
    Denied,
    /// The browser has explicitly granted local-file access.
    Granted,
}

/// Browser sandbox or container identity associated with a local-file capability.
///
/// The value is opaque to the security policy. Browser composition must use the
/// same value when creating the policy and when translating a request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileSandboxId(String);

impl FileSandboxId {
    /// Creates an opaque sandbox or container identity.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl Default for FileSandboxId {
    /// Returns the process-local default container identity.
    ///
    /// Browser composition must still explicitly provide a matching identity
    /// and a granted permission before a request can access a file.
    fn default() -> Self {
        Self::new("default-file-container")
    }
}

/// Security-relevant context for one attempted local-file access.
///
/// This initial vertical slice intentionally supports only explicitly granted
/// top-level navigations. Subresource access is denied until file-document
/// origin semantics are available end-to-end.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileRequestContext {
    /// Origin of the document or worker that initiated the request.
    pub initiating_origin: FileOrigin,
    /// Origin represented by the requested URL. Local-file requests use
    /// [`FileOrigin::LocalFile`].
    pub requested_origin: FileOrigin,
    /// Whether the request is a navigation or subresource access.
    pub kind: FileRequestKind,
    /// Permission state supplied by browser composition for this request.
    pub permission: FileAccessPermission,
    /// Sandbox or container identity supplied by browser composition.
    pub sandbox: FileSandboxId,
}
