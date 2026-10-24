//! Request-context primitives used to authorize browser `file:` access.

use std::sync::{Arc, Mutex};

/// Origin information supplied by the browser when it asks to access a local file.
///
/// This is deliberately a security-owned representation. Browser adapters must
/// translate their networking origin type before calling
/// [`crate::file_access::FileAccessCapability`].
/// Tuple origins are serialized in their canonical form and are retained for
/// policy decisions and auditing. File documents receive opaque origins, and
/// this policy denies local-file subresource access for every initiator until
/// browser document/container identity is available end-to-end.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileOrigin {
    /// The request has no initiating client origin.
    Absent,
    /// The request has an opaque initiating origin, including a file document
    /// or a sandboxed document.
    Opaque,
    /// A canonical tuple origin, such as `https://example.test`.
    Tuple(String),
    /// The requested resource is a local file, or the initiating document is
    /// represented as a local-file document by browser policy.
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

/// Opaque authorization issued by one [`crate::file_access::FileAccessCapability`].
///
/// Only the policy that issued this value accepts it. Browser composition must
/// retain grants exclusively in browser-owned state and never expose them to
/// request or web-content code. Dropping a grant source denies future opens.
#[derive(Clone, Debug)]
pub struct FileAccessGrant {
    pub(crate) capability_id: u64,
    session: FileAccessSession,
}

impl PartialEq for FileAccessGrant {
    fn eq(&self, other: &Self) -> bool {
        self.capability_id == other.capability_id
            && Arc::ptr_eq(&self.session.state, &other.session.state)
    }
}

impl Eq for FileAccessGrant {}

impl FileAccessGrant {
    pub(crate) fn new(capability_id: u64) -> Self {
        Self {
            capability_id,
            session: FileAccessSession::new(),
        }
    }

    /// Invalidates this grant's session and every lease derived from it.
    ///
    /// Browser-owned lifecycle code calls this when the associated permission
    /// ends. Invalidated grants cannot be made active again; issue a new grant
    /// from the capability when replacing a session.
    pub fn revoke(&self) {
        self.session.revoke();
    }

    /// Captures a generation-bound lease for one authorization and open.
    ///
    /// The lease remains current until this grant is revoked. Callers must
    /// validate it before exposing response metadata or body data.
    pub fn lease(&self) -> Option<FileAccessLease> {
        self.session.lease()
    }
}

/// Security-owned lifetime state for a browser-issued file-access grant.
///
/// This type is retained inside [`FileAccessGrant`]. Browser code observes it
/// only through generation-bound [`FileAccessLease`] values, so revocation
/// cannot be undone for an already-issued grant.
#[derive(Clone, Debug)]
struct FileAccessSession {
    state: Arc<Mutex<FileAccessSessionState>>,
}

#[derive(Debug)]
struct FileAccessSessionState {
    generation: u64,
    active: bool,
}

impl FileAccessSession {
    fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(FileAccessSessionState {
                generation: 0,
                active: true,
            })),
        }
    }

    fn revoke(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.active = false;
            state.generation = state.generation.wrapping_add(1);
        }
    }

    fn lease(&self) -> Option<FileAccessLease> {
        let state = self.state.lock().ok()?;
        state.active.then_some(FileAccessLease {
            session: self.clone(),
            generation: state.generation,
        })
    }
}

/// A generation-bound authorization lease for one local-file operation.
///
/// A lease is current only while the grant from which it was captured remains
/// active at the same session generation. It is safe to clone for setup and
/// stream tasks belonging to that one operation.
#[derive(Clone, Debug)]
pub struct FileAccessLease {
    session: FileAccessSession,
    generation: u64,
}

impl FileAccessLease {
    /// Returns whether this lease still belongs to an active grant generation.
    ///
    /// This check and [`FileAccessGrant::revoke`] serialize through the same
    /// security-owned session state. A caller that observes `true` may
    /// linearize response publication at that check; callers must recheck
    /// before each body-data delivery.
    pub fn is_current(&self) -> bool {
        self.session
            .state
            .lock()
            .is_ok_and(|state| state.active && state.generation == self.generation)
    }
}

/// Security-relevant context for one attempted local-file access.
///
/// This policy assigns every admitted file document an opaque origin. It
/// supports only top-level navigations authorized by a policy-issued
/// [`FileAccessGrant`]; subresources, workers, and nested contexts are denied
/// regardless of whether their initiating origin is local-file, tuple, opaque,
/// or absent.
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
