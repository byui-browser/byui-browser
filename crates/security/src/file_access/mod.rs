//! Policy and filesystem primitives for browser-authorized `file:` access.
//!
//! This module deliberately does not depend on the `net` crate. The browser
//! layer adapts these primitives to `net::FileProvider` when composing a
//! request controller.

mod error;
mod filesystem;
mod request;

pub use error::FileAccessError;
pub use filesystem::{FileResource, OpenedFile};
pub use request::{FileAccessGrant, FileOrigin, FileRequestContext, FileRequestKind};

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use cap_std::fs::Dir;

static NEXT_CAPABILITY_ID: AtomicU64 = AtomicU64::new(1);

/// A directory capability and its trusted, canonical selection path.
///
/// Platform integration creates this value immediately from a user-selected
/// directory handle (or another already-authorized directory capability). The
/// path and handle must describe that same selection. In particular, callers
/// must not construct this from a pathname and then reopen that pathname: that
/// reintroduces a root-replacement race. Holding a [`Dir`] is the authority;
/// the path is used only to translate a selected `file:` URL to a relative
/// name below that authority.
#[derive(Debug)]
pub struct TrustedFileAccessRoot {
    canonical_path: PathBuf,
    directory: Dir,
}

impl TrustedFileAccessRoot {
    /// Combines a trusted canonical selection path with its already-opened
    /// directory capability.
    ///
    /// `canonical_path` must be the absolute, canonical path reported by the
    /// same trusted platform operation that produced `directory`. This method
    /// deliberately performs no filesystem lookup, so it cannot reopen a
    /// replacement at that path. An empty or relative path is rejected.
    pub fn from_open_directory(
        canonical_path: PathBuf,
        directory: Dir,
    ) -> Result<Self, FileAccessError> {
        if !canonical_path.is_absolute() || canonical_path.as_os_str().is_empty() {
            return Err(FileAccessError::InvalidPath);
        }
        Ok(Self {
            canonical_path,
            directory,
        })
    }
}

/// Browser-owned capability for reading files below a fixed set of trusted roots.
///
/// Construct this value only at the browser or platform trust boundary, after a
/// user choice or equivalent embedder authorization has produced
/// [`TrustedFileAccessRoot`] values. Each root combines its selection path and
/// already-open directory authority, so this capability never separately
/// resolves and reopens a mutable root pathname. Web content and
/// request-construction code should receive requests and grants, never this
/// capability or its roots.
#[derive(Debug)]
pub struct FileAccessCapability {
    roots: Vec<TrustedFileAccessRoot>,
    capability_id: u64,
}

impl FileAccessCapability {
    /// Creates a local-file capability from roots supplied by trusted browser
    /// or platform code.
    ///
    /// Each root must already hold the directory capability captured by the
    /// platform selection operation. An empty list creates a default-deny
    /// capability. Callers must not pass paths supplied by web content or
    /// recreate directory handles from paths at this boundary.
    pub fn from_trusted_roots(
        allowed_roots: impl IntoIterator<Item = TrustedFileAccessRoot>,
    ) -> Result<Self, FileAccessError> {
        Ok(Self {
            roots: allowed_roots.into_iter().collect(),
            capability_id: next_capability_id(),
        })
    }

    /// Issues a grant accepted only by this capability for future file opens.
    ///
    /// The browser must retain the grant in browser-owned state and may revoke
    /// it when the associated permission lifetime ends.
    pub fn issue_grant(&self) -> FileAccessGrant {
        FileAccessGrant::new(self.capability_id)
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
        if grant.capability_id != self.capability_id
            || context.requested_origin != FileOrigin::LocalFile
            || context.kind != FileRequestKind::Navigation
        {
            return Err(FileAccessError::Denied);
        }
        Ok(())
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
        self.authorize_request(context, grant)?;
        let (root_index, relative) = self.authorize_path(requested)?;
        self.open_relative(root_index, &relative)
    }

    fn authorize_path(
        &self,
        requested: impl AsRef<Path>,
    ) -> Result<(usize, PathBuf), FileAccessError> {
        let canonical = fs::canonicalize(requested).map_err(FileAccessError::from)?;
        let Some((root_index, root)) = self
            .roots
            .iter()
            .enumerate()
            .filter(|(_, root)| canonical.starts_with(&root.canonical_path))
            .max_by_key(|(_, root)| root.canonical_path.components().count())
        else {
            return Err(FileAccessError::Denied);
        };
        let relative = canonical
            .strip_prefix(&root.canonical_path)
            .map(Path::to_path_buf)
            .map_err(|_| FileAccessError::InvalidPath)?;
        Ok((
            root_index,
            if relative.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                relative
            },
        ))
    }

    fn open_relative(
        &self,
        root_index: usize,
        relative: &Path,
    ) -> Result<OpenedFile, FileAccessError> {
        if relative == Path::new(".") {
            return Err(FileAccessError::UnsupportedFile);
        }
        let root = self.roots.get(root_index).ok_or(FileAccessError::Denied)?;
        let file = root
            .directory
            .open(relative)
            .map_err(FileAccessError::from)?;
        let file = file.into_std();
        let metadata = file.metadata().map_err(FileAccessError::from)?;
        if !metadata.file_type().is_file() {
            return Err(FileAccessError::UnsupportedFile);
        }
        Ok(OpenedFile {
            file,
            media_type: None,
            length: metadata.len(),
        })
    }
}

fn next_capability_id() -> u64 {
    NEXT_CAPABILITY_ID.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use cap_std::{ambient_authority, fs::Dir};

    use super::*;

    static NEXT_TEMP_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    fn temporary_directory() -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "byui-browser-file-access-capability-{}-{}",
            std::process::id(),
            NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        directory
    }

    fn trusted_root(path: &Path) -> TrustedFileAccessRoot {
        let canonical_path = fs::canonicalize(path).unwrap();
        let directory = Dir::open_ambient_dir(&canonical_path, ambient_authority()).unwrap();
        TrustedFileAccessRoot::from_open_directory(canonical_path, directory).unwrap()
    }

    fn navigation_context() -> FileRequestContext {
        FileRequestContext {
            initiating_origin: FileOrigin::Absent,
            requested_origin: FileOrigin::LocalFile,
            kind: FileRequestKind::Navigation,
        }
    }

    #[test]
    fn trusted_root_rejects_a_relative_selection_path() {
        let directory = temporary_directory();
        let handle = Dir::open_ambient_dir(&directory, ambient_authority()).unwrap();

        assert!(matches!(
            TrustedFileAccessRoot::from_open_directory(PathBuf::from("relative"), handle),
            Err(FileAccessError::InvalidPath)
        ));

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn capability_reads_only_from_its_captured_root() {
        let directory = temporary_directory();
        let allowed = directory.join("allowed");
        let allowed_file = allowed.join("page.html");
        let outside = directory.join("outside.html");
        fs::create_dir(&allowed).unwrap();
        fs::write(&allowed_file, b"allowed").unwrap();
        fs::write(&outside, b"denied").unwrap();
        let capability =
            FileAccessCapability::from_trusted_roots([trusted_root(&allowed)]).unwrap();
        let grant = capability.issue_grant();

        assert_eq!(
            capability
                .open_authorized_file(&navigation_context(), &grant, &allowed_file)
                .unwrap()
                .read_all()
                .unwrap()
                .bytes,
            b"allowed"
        );
        assert!(matches!(
            capability.open_authorized_file(&navigation_context(), &grant, &outside),
            Err(FileAccessError::Denied)
        ));

        drop(capability);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn capability_grants_are_bound_to_one_capability() {
        let first = FileAccessCapability::from_trusted_roots([]).unwrap();
        let second = FileAccessCapability::from_trusted_roots([]).unwrap();

        assert_eq!(
            first.authorize_request(&navigation_context(), &second.issue_grant()),
            Err(FileAccessError::Denied)
        );
    }

    #[test]
    fn request_authorization_requires_a_local_navigation() {
        let capability = FileAccessCapability::from_trusted_roots([]).unwrap();
        let grant = capability.issue_grant();

        for context in [
            FileRequestContext {
                initiating_origin: FileOrigin::Absent,
                requested_origin: FileOrigin::Opaque,
                kind: FileRequestKind::Navigation,
            },
            FileRequestContext {
                initiating_origin: FileOrigin::Tuple("https://example.test".into()),
                requested_origin: FileOrigin::LocalFile,
                kind: FileRequestKind::Subresource,
            },
        ] {
            assert_eq!(
                capability.authorize_request(&context, &grant),
                Err(FileAccessError::Denied)
            );
        }
    }

    #[test]
    fn denied_requests_do_not_canonicalize_their_path() {
        let capability = FileAccessCapability::from_trusted_roots([]).unwrap();
        let another = FileAccessCapability::from_trusted_roots([]).unwrap();

        assert!(matches!(
            capability.open_authorized_file(
                &navigation_context(),
                &another.issue_grant(),
                "this-path-does-not-need-to-exist",
            ),
            Err(FileAccessError::Denied)
        ));
    }

    #[test]
    fn root_path_is_not_served_as_a_regular_file() {
        let directory = temporary_directory();
        let capability =
            FileAccessCapability::from_trusted_roots([trusted_root(&directory)]).unwrap();
        let grant = capability.issue_grant();

        assert!(matches!(
            capability.open_authorized_file(&navigation_context(), &grant, &directory),
            Err(FileAccessError::UnsupportedFile)
        ));

        drop(capability);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn most_specific_captured_root_is_selected() {
        let directory = temporary_directory();
        let parent = directory.join("parent");
        let child = parent.join("child");
        let file = child.join("page.html");
        fs::create_dir(&parent).unwrap();
        fs::create_dir(&child).unwrap();
        fs::write(&file, b"nested").unwrap();
        let capability =
            FileAccessCapability::from_trusted_roots([trusted_root(&parent), trusted_root(&child)])
                .unwrap();

        assert_eq!(capability.authorize_path(&file).unwrap().0, 1);

        drop(capability);
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_denied_before_the_captured_root_is_opened() {
        use std::os::unix::fs::symlink;

        let directory = temporary_directory();
        let allowed = directory.join("allowed");
        let outside = directory.join("outside");
        let secret = outside.join("secret.txt");
        fs::create_dir(&allowed).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(&secret, b"secret").unwrap();
        symlink(&outside, allowed.join("escape")).unwrap();
        let capability =
            FileAccessCapability::from_trusted_roots([trusted_root(&allowed)]).unwrap();
        let grant = capability.issue_grant();

        assert!(matches!(
            capability.open_authorized_file(
                &navigation_context(),
                &grant,
                allowed.join("escape").join("secret.txt"),
            ),
            Err(FileAccessError::Denied)
        ));

        drop(capability);
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn opened_file_remains_bound_to_its_original_handle_after_path_replacement() {
        let directory = temporary_directory();
        let path = directory.join("page.html");
        let replacement = directory.join("replacement.html");
        fs::write(&path, b"original").unwrap();
        fs::write(&replacement, b"replacement").unwrap();
        let capability =
            FileAccessCapability::from_trusted_roots([trusted_root(&directory)]).unwrap();
        let grant = capability.issue_grant();
        let opened = capability
            .open_authorized_file(&navigation_context(), &grant, &path)
            .unwrap();

        fs::rename(&replacement, &path).unwrap();

        assert_eq!(opened.read_all().unwrap().bytes, b"original");

        drop(capability);
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn captured_directory_handle_survives_root_path_replacement() {
        let directory = temporary_directory();
        let selected = directory.join("selected");
        let replacement = directory.join("replacement");
        let previous = directory.join("previous");
        fs::create_dir(&selected).unwrap();
        fs::create_dir(&replacement).unwrap();
        fs::write(selected.join("page.html"), b"selected root").unwrap();
        fs::write(replacement.join("page.html"), b"replacement root").unwrap();
        let capability =
            FileAccessCapability::from_trusted_roots([trusted_root(&selected)]).unwrap();
        let grant = capability.issue_grant();

        fs::rename(&selected, &previous).unwrap();
        fs::rename(&replacement, &selected).unwrap();

        assert_eq!(
            capability
                .open_authorized_file(&navigation_context(), &grant, selected.join("page.html"))
                .unwrap()
                .read_all()
                .unwrap()
                .bytes,
            b"selected root"
        );

        drop(capability);
        fs::remove_dir_all(directory).unwrap();
    }
}
