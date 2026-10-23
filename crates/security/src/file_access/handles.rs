use std::{fs, io::Read, path::PathBuf};

use cap_std::fs::Dir;

use super::FileAccessError;

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
    pub(super) canonical_path: PathBuf,
    pub(super) directory: Dir,
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

/// An opened regular file accepted by the capability-backed filesystem.
///
/// The handle is intentionally private and supports chunked reads through the
/// same file selected by the capability. Callers must not reopen the
/// authorized pathname.
#[derive(Debug)]
pub struct OpenedFile {
    pub(crate) file: fs::File,
    pub(crate) media_type: Option<String>,
    pub(crate) length: u64,
}

impl OpenedFile {
    /// Reads the next chunk through this opened handle.
    ///
    /// The returned count is zero at end of file. The operation is blocking;
    /// callers running on an asynchronous executor must perform it on a
    /// blocking executor. The read always uses the retained handle and never
    /// reopens the authorized pathname.
    pub fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, FileAccessError> {
        self.file.read(buffer).map_err(FileAccessError::from)
    }

    /// Returns the file length observed when the handle was opened.
    pub fn length(&self) -> u64 {
        self.length
    }

    /// Returns the filesystem-provided media type, when one is available.
    pub fn media_type(&self) -> Option<&str> {
        self.media_type.as_deref()
    }
}
