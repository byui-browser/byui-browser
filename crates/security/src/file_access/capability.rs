use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[cfg(windows)]
use std::fs;

use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};

use super::{
    FileAccessError, FileAccessGrant, FileAccessLease, FileOrigin, FileRequestContext,
    FileRequestKind, OpenedFile, TrustedDirectorySelection,
};

static NEXT_CAPABILITY_ID: AtomicU64 = AtomicU64::new(1);

/// Browser-owned capability for reading files below trusted directory selections.
///
/// Construct this value only at the browser or platform trust boundary, after a
/// user choice or equivalent embedder authorization has produced
/// [`TrustedDirectorySelection`] values. Each selection combines its path and
/// already-open directory authority, so this capability never separately
/// resolves and reopens a mutable root pathname. Web content and
/// request-construction code should receive requests and grants, never this
/// capability or its roots.
#[derive(Debug)]
pub struct FileAccessCapability {
    roots: Vec<TrustedDirectorySelection>,
    capability_id: u64,
}

impl FileAccessCapability {
    /// Creates a local-file capability from selections supplied by trusted
    /// platform code.
    ///
    /// Each selection must already hold the directory capability captured by the
    /// platform selection operation. An empty list creates a default-deny
    /// capability. Callers must not pass paths supplied by web content or
    /// recreate directory handles from paths at this boundary.
    pub fn from_trusted_directory_selections(
        selected_directories: impl IntoIterator<Item = TrustedDirectorySelection>,
    ) -> Result<Self, FileAccessError> {
        Ok(Self {
            roots: selected_directories.into_iter().collect(),
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
    pub(crate) fn open_authorized_file(
        &self,
        context: &FileRequestContext,
        grant: &FileAccessGrant,
        requested: impl AsRef<std::path::Path>,
    ) -> Result<OpenedFile, FileAccessError> {
        self.authorize_request(context, grant)?;
        let (root_index, relative) = self.authorize_path(requested)?;
        self.open_relative(root_index, &relative)
    }

    /// Authorizes and opens one file while a session lease remains current.
    ///
    /// Browser providers use this operation for revocable sessions. It checks
    /// `lease` before filesystem work and once more before returning the
    /// opened handle, so a revocation concurrent with setup cannot admit that
    /// handle for response publication. Callers must still validate the lease
    /// immediately before exposing metadata and each body chunk.
    pub fn open_authorized_file_with_lease(
        &self,
        context: &FileRequestContext,
        grant: &FileAccessGrant,
        lease: &FileAccessLease,
        requested: impl AsRef<std::path::Path>,
    ) -> Result<OpenedFile, FileAccessError> {
        if !lease.is_current() {
            return Err(FileAccessError::Denied);
        }
        let opened = self.open_authorized_file(context, grant, requested)?;
        if !lease.is_current() {
            return Err(FileAccessError::Denied);
        }
        Ok(opened)
    }

    pub(super) fn authorize_path(
        &self,
        requested: impl AsRef<Path>,
    ) -> Result<(usize, PathBuf), FileAccessError> {
        // Establish root eligibility without touching the filesystem. This
        // prevents a caller from learning whether an ineligible path exists.
        let lexical = normalize_absolute_path(requested.as_ref())?;
        if let Some((root_index, root)) = self
            .roots
            .iter()
            .enumerate()
            .filter_map(|(root_index, root)| {
                let root_components = normalize_absolute_path(&root.canonical_path).ok()?;
                lexical
                    .starts_with(&root_components)
                    .then_some((root_index, root_components))
            })
            .max_by_key(|(_, root_components)| root_components.len())
        {
            return Ok((root_index, relative_path(&lexical[root.len()..])?));
        }

        self.authorize_windows_alias(requested.as_ref())
    }

    /// Resolves a Windows path spelling that cannot be compared lexically.
    ///
    /// Long-name, short-name, and extended-length spellings can identify a
    /// selected directory without sharing the canonical path's components.
    /// This fallback is unavailable on other platforms, where a lexical miss
    /// is always denied without a filesystem lookup.
    #[cfg(windows)]
    fn authorize_windows_alias(
        &self,
        requested: &Path,
    ) -> Result<(usize, PathBuf), FileAccessError> {
        let mut unresolved = Vec::new();
        let mut candidate = requested;
        let canonical = loop {
            match fs::canonicalize(candidate) {
                Ok(canonical) => break canonical,
                Err(_) => {
                    let name = candidate.file_name().ok_or(FileAccessError::Denied)?;
                    unresolved.push(name.to_owned());
                    candidate = candidate.parent().ok_or(FileAccessError::Denied)?;
                }
            }
        };
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
        let mut relative = if relative.as_os_str().is_empty() {
            PathBuf::new()
        } else {
            relative
        };
        for component in unresolved.iter().rev() {
            relative.push(component);
        }
        Ok((
            root_index,
            if relative.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                relative
            },
        ))
    }

    #[cfg(not(windows))]
    fn authorize_windows_alias(
        &self,
        _requested: &Path,
    ) -> Result<(usize, PathBuf), FileAccessError> {
        Err(FileAccessError::Denied)
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
            media_type: media_type_for_path(relative).map(str::to_owned),
            length: metadata.len(),
        })
    }
}

/// Normalizes path syntax without resolving filesystem objects.
///
/// Paths matching a trusted root are opened from the retained root handle
/// without canonicalization. Paths outside a trusted root receive the same
/// denial regardless of whether they exist.
#[derive(Clone, Debug, Eq, PartialEq)]
enum LexicalComponent {
    Prefix(std::ffi::OsString),
    Root,
    Normal(std::ffi::OsString),
}

fn normalize_absolute_path(path: &Path) -> Result<Vec<LexicalComponent>, FileAccessError> {
    if !path.is_absolute() {
        return Err(FileAccessError::InvalidPath);
    }

    let mut normalized = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(prefix) => {
                let prefix = prefix.as_os_str().to_string_lossy().into_owned();
                let prefix = prefix.strip_prefix("\\\\?\\").unwrap_or(&prefix);
                normalized.push(LexicalComponent::Prefix(prefix.into()))
            }
            std::path::Component::RootDir => normalized.push(LexicalComponent::Root),
            std::path::Component::CurDir => {}
            std::path::Component::Normal(name) => {
                normalized.push(LexicalComponent::Normal(name.to_owned()))
            }
            std::path::Component::ParentDir => {
                if matches!(normalized.last(), Some(LexicalComponent::Normal(_))) {
                    normalized.pop();
                }
            }
        }
    }

    if normalized
        .iter()
        .any(|component| matches!(component, LexicalComponent::Root))
    {
        Ok(normalized)
    } else {
        Err(FileAccessError::InvalidPath)
    }
}

/// Builds a retained-root-relative path from normalized lexical components.
fn relative_path(components: &[LexicalComponent]) -> Result<PathBuf, FileAccessError> {
    let mut relative = PathBuf::new();
    for component in components {
        let LexicalComponent::Normal(name) = component else {
            return Err(FileAccessError::InvalidPath);
        };
        relative.push(name);
    }
    Ok(if relative.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        relative
    })
}

/// Returns the safe, deterministic media type associated with a file name.
///
/// This intentionally relies only on an allowlisted filename extension. It
/// does not inspect the file contents, so caller-controlled bytes cannot
/// influence response metadata.
pub(super) fn media_type_for_path(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?;
    if extension.eq_ignore_ascii_case("html") || extension.eq_ignore_ascii_case("htm") {
        Some("text/html; charset=utf-8")
    } else if extension.eq_ignore_ascii_case("css") {
        Some("text/css; charset=utf-8")
    } else if extension.eq_ignore_ascii_case("js") || extension.eq_ignore_ascii_case("mjs") {
        Some("text/javascript; charset=utf-8")
    } else if extension.eq_ignore_ascii_case("json") {
        Some("application/json")
    } else if extension.eq_ignore_ascii_case("png") {
        Some("image/png")
    } else if extension.eq_ignore_ascii_case("jpg") || extension.eq_ignore_ascii_case("jpeg") {
        Some("image/jpeg")
    } else if extension.eq_ignore_ascii_case("gif") {
        Some("image/gif")
    } else if extension.eq_ignore_ascii_case("svg") {
        Some("image/svg+xml")
    } else if extension.eq_ignore_ascii_case("webp") {
        Some("image/webp")
    } else {
        None
    }
}

/// Opens a regular-file candidate one component at a time beneath `root`.
///
/// Every component is opened relative to a handle retained from the trusted
/// root selection, after explicitly rejecting symlinks and with symlink
/// following disabled. This prevents a component replacement after pathname
/// authorization from redirecting the eventual open outside the selected tree.
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
        if directory
            .symlink_metadata(name)
            .map_err(FileAccessError::from)?
            .file_type()
            .is_symlink()
        {
            return Err(FileAccessError::Denied);
        }
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
