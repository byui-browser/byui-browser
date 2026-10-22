//! Request-context primitives used to authorize browser `file:` access.

/// Origin information supplied by the browser when it asks to access a local file.
///
/// This is deliberately a security-owned representation. Browser adapters must
/// translate their networking origin type before calling [`FileAccessCapability`].
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

/// Opaque authorization issued by one [`FileAccessCapability`].
///
/// Only the policy that issued this value accepts it. Browser composition must
/// retain grants exclusively in browser-owned state and never expose them to
/// request or web-content code. Dropping a grant source denies future opens.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileAccessGrant {
    pub(crate) capability_id: u64,
}

impl FileAccessGrant {
    pub(crate) fn new(capability_id: u64) -> Self {
        Self { capability_id }
    }
}

/// Security-relevant context for one attempted local-file access.
///
/// This initial vertical slice intentionally supports only top-level
/// navigations authorized by a policy-issued [`FileAccessGrant`]. Subresource
/// access is denied until file-document origin semantics are available
/// end-to-end.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileRequestContext {
    /// Origin of the document or worker that initiated the request.
    pub initiating_origin: FileOrigin,
    /// Origin represented by the requested URL. Local-file requests use
    /// [`FileOrigin::LocalFile`].
    pub requested_origin: FileOrigin,
    /// Whether the request is a navigation or subresource access.
    pub kind: FileRequestKind,
}
