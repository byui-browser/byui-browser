//! Path authorization and sandbox-boundary checks.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use super::CapFileSystem;
use super::{
    FileAccessError, FileAccessGrant, FileOrigin, FileRequestContext, FileRequestKind, OpenedFile,
};

static NEXT_POLICY_ID: AtomicU64 = AtomicU64::new(1);

/// A filesystem path that has passed the configured access policy.
///
/// The path is canonicalized before construction and can only be created by
/// [`FileAccessPolicy`]. Callers should retain this value instead of
/// re-authorizing or reconstructing the path from an untrusted URL.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AuthorizedPath {
    canonical: PathBuf,
    root: PathBuf,
    relative: PathBuf,
}

impl AuthorizedPath {
    /// Returns the canonical path approved by the policy.
    #[cfg(test)]
    pub(crate) fn as_path(&self) -> &Path {
        &self.canonical
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn relative(&self) -> &Path {
        &self.relative
    }
}

/// Default-deny policy for browser-authorized local files.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileAccessPolicy {
    allowed_roots: Vec<PathBuf>,
    policy_id: u64,
}

impl Default for FileAccessPolicy {
    /// Creates a policy that denies every path and issues its own grants.
    fn default() -> Self {
        Self {
            allowed_roots: Vec::new(),
            policy_id: next_policy_id(),
        }
    }
}

impl FileAccessPolicy {
    /// Creates a policy allowing reads below `allowed_roots`.
    ///
    /// Roots are canonicalized when the policy is created. An empty root list
    /// denies every request.
    pub fn new(allowed_roots: impl IntoIterator<Item = PathBuf>) -> Result<Self, FileAccessError> {
        let allowed_roots = allowed_roots
            .into_iter()
            .map(|root| fs::canonicalize(root).map_err(FileAccessError::from))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            allowed_roots,
            policy_id: next_policy_id(),
        })
    }

    /// Issues a grant accepted only by this policy.
    ///
    /// The caller that receives this grant is the trusted browser/embedder
    /// boundary. It must retain the grant in browser-owned state and must not
    /// expose it to web-content or request-construction code.
    pub fn issue_grant(&self) -> FileAccessGrant {
        FileAccessGrant::new(self.policy_id)
    }

    /// Authorizes the browser context before any filesystem path is opened.
    ///
    /// Access is default-deny. The caller must provide a grant issued by this
    /// policy and a local-file navigation target. This initial policy supports
    /// only top-level navigations; subresource access remains denied until
    /// file-document origin semantics are defined.
    pub(crate) fn authorize_request(
        &self,
        context: &FileRequestContext,
        grant: &FileAccessGrant,
    ) -> Result<(), FileAccessError> {
        if grant.policy_id != self.policy_id
            || context.requested_origin != FileOrigin::LocalFile
            || context.kind != FileRequestKind::Navigation
        {
            return Err(FileAccessError::Denied);
        }
        Ok(())
    }

    /// Authorizes and canonicalizes one requested path.
    ///
    /// A request is allowed only when its canonical path is the configured
    /// root or a descendant of that root. Canonicalization resolves `..` and
    /// symlinks before the containment check, so a link that resolves outside
    /// an allowed root is denied.
    ///
    /// This check is not an atomic filesystem-containment operation. It remains
    /// crate-internal so callers use [`Self::open_authorized_file`], which
    /// preserves both request-policy and capability boundaries.
    pub(crate) fn authorize_path(
        &self,
        requested: impl AsRef<Path>,
    ) -> Result<AuthorizedPath, FileAccessError> {
        let canonical = fs::canonicalize(requested).map_err(FileAccessError::from)?;
        let Some(root) = self
            .allowed_roots
            .iter()
            .filter(|root| is_within_root(&canonical, root))
            .max_by_key(|root| root.components().count())
        else {
            return Err(FileAccessError::Denied);
        };
        let relative = canonical
            .strip_prefix(root)
            .map(Path::to_path_buf)
            .map_err(|_| FileAccessError::InvalidPath)?;
        Ok(AuthorizedPath {
            canonical,
            root: root.clone(),
            relative: if relative.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                relative
            },
        })
    }

    /// Authorizes one browser request and opens its requested file capability.
    ///
    /// Request authorization runs before path canonicalization or filesystem
    /// access. The returned [`OpenedFile`] owns the handle that callers must
    /// use for subsequent reads. This prevents callers from authorizing one
    /// pathname and later reopening that pathname for its contents.
    pub fn open_authorized_file(
        &self,
        context: &FileRequestContext,
        grant: &FileAccessGrant,
        requested: impl AsRef<Path>,
        filesystem: &CapFileSystem,
    ) -> Result<OpenedFile, FileAccessError> {
        self.authorize_request(context, grant)?;
        let path = self.authorize_path(requested)?;
        filesystem.open(&path)
    }
}

/// Returns whether `candidate` is a root or descendant of `root`.
///
/// [`Path::starts_with`] compares path components rather than string prefixes,
/// which keeps similarly named sibling directories distinct (for example,
/// `/allowed` and `/allowed-sibling`). Both paths are canonicalized by the
/// policy before this helper is used.
fn is_within_root(candidate: &Path, root: &Path) -> bool {
    candidate.starts_with(root)
}

fn next_policy_id() -> u64 {
    NEXT_POLICY_ID.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use super::*;

    static NEXT_TEMP_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    fn temporary_directory() -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "byui-browser-file-access-policy-{}-{}",
            std::process::id(),
            NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        directory
    }

    #[test]
    fn empty_policy_denies_existing_paths() {
        let policy = FileAccessPolicy::default();
        assert_eq!(
            policy.authorize_path(std::env::temp_dir()),
            Err(FileAccessError::Denied)
        );
    }

    #[test]
    fn request_authorization_requires_a_policy_grant_and_navigation() {
        let policy = FileAccessPolicy::new([]).unwrap();
        let grant = policy.issue_grant();
        let allowed = FileRequestContext {
            initiating_origin: FileOrigin::Tuple("https://example.test".into()),
            requested_origin: FileOrigin::LocalFile,
            kind: FileRequestKind::Navigation,
        };

        assert_eq!(policy.authorize_request(&allowed, &grant), Ok(()));

        for context in [
            FileRequestContext {
                kind: FileRequestKind::Subresource,
                ..allowed.clone()
            },
            FileRequestContext {
                requested_origin: FileOrigin::Opaque,
                ..allowed.clone()
            },
        ] {
            assert_eq!(
                policy.authorize_request(&context, &grant),
                Err(FileAccessError::Denied)
            );
        }

        let other_policy = FileAccessPolicy::new([]).unwrap();
        assert_eq!(
            policy.authorize_request(&allowed, &other_policy.issue_grant()),
            Err(FileAccessError::Denied)
        );
    }

    #[test]
    fn denied_request_precedes_path_canonicalization() {
        let directory = temporary_directory();
        let policy = FileAccessPolicy::new([directory.clone()]).unwrap();
        let filesystem = CapFileSystem::new([directory.clone()]).unwrap();
        let context = FileRequestContext {
            initiating_origin: FileOrigin::Absent,
            requested_origin: FileOrigin::LocalFile,
            kind: FileRequestKind::Navigation,
        };
        let foreign_policy = FileAccessPolicy::new([]).unwrap();

        assert!(matches!(
            policy.open_authorized_file(
                &context,
                &foreign_policy.issue_grant(),
                directory.join("missing.txt"),
                &filesystem,
            ),
            Err(FileAccessError::Denied)
        ));

        drop(filesystem);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn authorizes_allowed_roots_and_descendants_but_not_similarly_named_siblings() {
        let directory = temporary_directory();
        let allowed_root = directory.join("allowed");
        let sibling_root = directory.join("allowed-sibling");
        let nested_file = allowed_root.join("nested").join("file.txt");
        let sibling_file = sibling_root.join("file.txt");
        fs::create_dir(&allowed_root).unwrap();
        fs::create_dir_all(nested_file.parent().unwrap()).unwrap();
        fs::create_dir(&sibling_root).unwrap();
        fs::write(&nested_file, b"allowed").unwrap();
        fs::write(&sibling_file, b"denied").unwrap();
        let policy = FileAccessPolicy::new([allowed_root.clone()]).unwrap();

        assert_eq!(
            policy.authorize_path(&allowed_root).unwrap().as_path(),
            fs::canonicalize(&allowed_root).unwrap()
        );
        assert_eq!(
            policy.authorize_path(&nested_file).unwrap().as_path(),
            fs::canonicalize(&nested_file).unwrap()
        );
        assert_eq!(
            policy.authorize_path(&sibling_file),
            Err(FileAccessError::Denied)
        );

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn resolves_traversal_before_authorizing_absolute_paths() {
        let directory = temporary_directory();
        let allowed_root = directory.join("allowed");
        let nested_directory = allowed_root.join("nested");
        let allowed_file = allowed_root.join("allowed.txt");
        let outside_file = directory.join("outside.txt");
        fs::create_dir(&allowed_root).unwrap();
        fs::create_dir(&nested_directory).unwrap();
        fs::write(&allowed_file, b"allowed").unwrap();
        fs::write(&outside_file, b"denied").unwrap();
        let policy = FileAccessPolicy::new([allowed_root.clone()]).unwrap();

        assert_eq!(
            policy
                .authorize_path(nested_directory.join("..").join("allowed.txt"))
                .unwrap()
                .as_path(),
            fs::canonicalize(&allowed_file).unwrap()
        );
        assert_eq!(
            policy.authorize_path(allowed_root.join("..").join("outside.txt")),
            Err(FileAccessError::Denied)
        );
        assert_eq!(
            policy.authorize_path(&outside_file),
            Err(FileAccessError::Denied)
        );

        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn denies_symlinks_that_resolve_outside_the_allowed_root() {
        use std::os::unix::fs::symlink;

        let directory = temporary_directory();
        let allowed_root = directory.join("allowed");
        let outside_directory = directory.join("outside");
        let outside_file = outside_directory.join("file.txt");
        fs::create_dir(&allowed_root).unwrap();
        fs::create_dir(&outside_directory).unwrap();
        fs::write(&outside_file, b"denied").unwrap();
        symlink(&outside_file, allowed_root.join("file-link.txt")).unwrap();
        symlink(&outside_directory, allowed_root.join("directory-link")).unwrap();
        let policy = FileAccessPolicy::new([allowed_root.clone()]).unwrap();

        assert_eq!(
            policy.authorize_path(allowed_root.join("file-link.txt")),
            Err(FileAccessError::Denied)
        );
        assert_eq!(
            policy.authorize_path(allowed_root.join("directory-link").join("file.txt")),
            Err(FileAccessError::Denied)
        );

        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn containment_distinguishes_windows_drive_and_unc_path_components() {
        assert!(is_within_root(
            std::path::Path::new(r"C:\allowed\file.txt"),
            std::path::Path::new(r"C:\allowed")
        ));
        assert!(!is_within_root(
            std::path::Path::new(r"C:\allowed-sibling\file.txt"),
            std::path::Path::new(r"C:\allowed")
        ));
        assert!(!is_within_root(
            std::path::Path::new(r"D:\allowed\file.txt"),
            std::path::Path::new(r"C:\allowed")
        ));
        assert!(is_within_root(
            std::path::Path::new(r"\\server\share\allowed\file.txt"),
            std::path::Path::new(r"\\server\share\allowed")
        ));
        assert!(!is_within_root(
            std::path::Path::new(r"\\server\share\allowed-sibling\file.txt"),
            std::path::Path::new(r"\\server\share\allowed")
        ));
        assert!(!is_within_root(
            std::path::Path::new(r"\\server\other-share\allowed\file.txt"),
            std::path::Path::new(r"\\server\share\allowed")
        ));
    }
}
