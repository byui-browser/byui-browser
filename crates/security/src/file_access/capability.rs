use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};

use super::{
    FileAccessError, FileAccessGrant, FileOrigin, FileRequestContext, FileRequestKind, OpenedFile,
    TrustedFileAccessRoot,
};

static NEXT_CAPABILITY_ID: AtomicU64 = AtomicU64::new(1);

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

    pub(super) fn authorize_path(
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
        let file = open_relative_nofollow(&root.directory, relative)?;
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

/// Opens a regular-file candidate one component at a time beneath `root`.
///
/// Every component is opened relative to a handle retained from the trusted
/// root selection, with symlink following disabled. This prevents a component
/// replacement after pathname authorization from redirecting the eventual
/// open outside the selected tree.
pub(super) fn open_relative_nofollow(
    root: &Dir,
    relative: &Path,
) -> Result<cap_std::fs::File, FileAccessError> {
    let mut directory = root.try_clone().map_err(FileAccessError::from)?;
    let mut components = relative.components().peekable();
    let mut final_file = None;

    while let Some(component) = components.next() {
        let std::path::Component::Normal(name) = component else {
            return Err(FileAccessError::InvalidPath);
        };
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let opened = directory
            .open_with(name, &options)
            .map_err(FileAccessError::from)?;

        if components.peek().is_some() {
            if !opened
                .metadata()
                .map_err(FileAccessError::from)?
                .file_type()
                .is_dir()
            {
                return Err(FileAccessError::UnsupportedFile);
            }
            directory = Dir::from_std_file(opened.into_std());
        } else {
            final_file = Some(opened);
        }
    }

    final_file.ok_or(FileAccessError::InvalidPath)
}

fn next_capability_id() -> u64 {
    NEXT_CAPABILITY_ID.fetch_add(1, Ordering::Relaxed)
}
