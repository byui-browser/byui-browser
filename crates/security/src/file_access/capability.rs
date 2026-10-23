use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};

use super::{
    FileAccessError, FileAccessGrant, FileOrigin, FileRequestContext, FileRequestKind, OpenedFile,
    TrustedDirectorySelection,
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
        // Establish root eligibility without touching the filesystem. This
        // prevents a caller from learning whether an ineligible path exists
        // based on the error returned by canonicalize.
        let lexical = normalize_absolute_path(requested.as_ref())?;
        let lexical_root = self
            .roots
            .iter()
            .enumerate()
            .filter(|(_, root)| {
                normalize_absolute_path(&root.canonical_path)
                    .map(|root| lexical.starts_with(&root))
                    .unwrap_or(false)
            })
            .max_by_key(|(_, root)| root.canonical_path.components().count());

        let canonical = match fs::canonicalize(requested.as_ref()) {
            Ok(canonical) => canonical,
            Err(error) if lexical_root.is_none() => {
                // A Windows short-name path can be inside a root even when
                // lexical components do not match the canonical long-name
                // root. Resolve only to normalize that alias. Every failure
                // in this fallback remains a denial, so it cannot disclose
                // whether an unauthorized path exists.
                let parent = requested.as_ref().parent().ok_or(FileAccessError::Denied)?;
                let canonical_parent =
                    fs::canonicalize(parent).map_err(|_| FileAccessError::Denied)?;
                if !self
                    .roots
                    .iter()
                    .any(|root| canonical_parent.starts_with(&root.canonical_path))
                {
                    return Err(FileAccessError::Denied);
                }
                return Err(FileAccessError::from(error));
            }
            Err(error) => return Err(FileAccessError::from(error)),
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
        if !canonical.starts_with(&root.canonical_path) {
            return Err(FileAccessError::Denied);
        }
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
            media_type: media_type_for_path(relative).map(str::to_owned),
            length: metadata.len(),
        })
    }
}

/// Normalizes path syntax without resolving filesystem objects.
///
/// This is deliberately performed before canonicalization so paths outside a
/// trusted root receive the same denial regardless of whether they exist.
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

    if normalized.first().is_some_and(|component| {
        matches!(
            component,
            LexicalComponent::Root | LexicalComponent::Prefix(_)
        )
    }) {
        Ok(normalized)
    } else {
        Err(FileAccessError::InvalidPath)
    }
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
