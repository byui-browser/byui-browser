use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
};

use cap_fs_ext::MetadataExt;
use cap_std::{ambient_authority, fs::Dir};

use super::FileAccessError;

/// An opaque directory selection approved by trusted platform code.
///
/// Platform integration creates this value immediately from a user-approved
/// directory handle (or another already-authorized directory capability). The
/// path and handle must describe that same selection. Browser configuration
/// consumes this value without accessing either part directly. Holding a
/// [`Dir`] is the authority; the path is used only to translate a selected
/// `file:` URL to a relative name below that authority.
#[derive(Debug)]
pub struct TrustedDirectorySelection {
    pub(super) canonical_path: PathBuf,
    pub(super) directory: Dir,
}

impl TrustedDirectorySelection {
    /// Creates a selection after verifying one trusted platform approval result.
    ///
    /// `canonical_path` must be the absolute, canonical path reported by the
    /// same trusted platform operation that produced `directory`. The method
    /// verifies that the pathname and retained directory handle identify the
    /// same filesystem object before storing either one. A mismatched pair is
    /// denied, rather than allowing path authorization for one directory and
    /// reads through another directory's handle.
    ///
    /// Platform code must capture `directory` from the user-approved selection
    /// operation and pass it here immediately. It must not reopen a pathname
    /// after approval: the retained handle is the authority used for later
    /// reads. This constructor necessarily consults the supplied path once to
    /// verify the pair; after construction, file access never reopens that
    /// root pathname. An empty, relative, or non-canonical path is rejected.
    pub fn from_verified_open_directory(
        canonical_path: PathBuf,
        directory: Dir,
    ) -> Result<Self, FileAccessError> {
        if !canonical_path.is_absolute() || canonical_path.as_os_str().is_empty() {
            return Err(FileAccessError::InvalidPath);
        }

        let resolved_path = fs::canonicalize(&canonical_path).map_err(FileAccessError::from)?;
        if resolved_path != canonical_path {
            return Err(FileAccessError::InvalidPath);
        }
        if !same_directory(&canonical_path, &directory)? {
            return Err(FileAccessError::Denied);
        }

        Ok(Self {
            canonical_path,
            directory,
        })
    }
}

/// Compares the filesystem identities observed through the supplied pathname
/// and the retained directory handle.
///
/// The pathname is used only while accepting a trusted selection. All later
/// file opens are relative to the retained handle.
fn same_directory(
    canonical_path: &std::path::Path,
    directory: &Dir,
) -> Result<bool, FileAccessError> {
    let path_metadata = Dir::open_ambient_dir(canonical_path, ambient_authority())
        .and_then(|path_directory| path_directory.metadata("."))
        .map_err(FileAccessError::from)?;
    let directory_metadata = directory.metadata(".").map_err(FileAccessError::from)?;

    if !path_metadata.is_dir() || !directory_metadata.is_dir() {
        return Ok(false);
    }

    Ok(path_metadata.dev() == directory_metadata.dev()
        && path_metadata.ino() == directory_metadata.ino())
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

    /// Reads up to `buffer.len()` leading bytes without changing the next
    /// chunk returned by [`Self::read_chunk`].
    ///
    /// Browser policy may use this bounded inspection to reject native
    /// executable formats before publishing response metadata. The operation
    /// is blocking and continues to use this retained authorized handle.
    pub fn read_prefix(&mut self, buffer: &mut [u8]) -> Result<usize, FileAccessError> {
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(FileAccessError::from)?;
        let count = self.file.read(buffer).map_err(FileAccessError::from)?;
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(FileAccessError::from)?;
        Ok(count)
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
