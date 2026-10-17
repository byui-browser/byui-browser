//! Filesystem abstraction used to open files approved by security policy.

use std::{fs, io::Read, sync::Arc};

use super::{AuthorizedPath, FileAccessError};

/// Materialized data read from an authorized file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileResource {
    /// File contents in byte order.
    pub bytes: Vec<u8>,
    /// Optional media type determined by the filesystem owner.
    pub media_type: Option<String>,
}

/// An opened regular file that was accepted by a [`FileSystem`] backend.
///
/// The handle is intentionally private. Callers can materialize it with
/// [`Self::read_all`] now, while a future streaming API can retain ownership of
/// the same handle. Callers must not reopen the authorized pathname.
#[derive(Debug)]
pub struct OpenedFile {
    file: fs::File,
    media_type: Option<String>,
}

impl OpenedFile {
    /// Reads the complete file through this opened handle.
    ///
    /// Consumes the handle, closing it after the read completes or fails. The
    /// returned bytes are therefore from the file that the backend opened,
    /// rather than from a later pathname lookup.
    pub fn read_all(mut self) -> Result<FileResource, FileAccessError> {
        let mut bytes = Vec::new();
        self.file
            .read_to_end(&mut bytes)
            .map_err(FileAccessError::from)?;
        Ok(FileResource {
            bytes,
            media_type: self.media_type,
        })
    }
}

/// Opens files after authorization has produced an [`AuthorizedPath`].
pub trait FileSystem: Send + Sync {
    /// Opens one authorized regular file and transfers ownership of its handle.
    ///
    /// Implementations must reject unsupported file types and must not return
    /// a handle that will later be replaced by a pathname-based read.
    fn open(&self, path: &AuthorizedPath) -> Result<OpenedFile, FileAccessError>;
}

/// Standard-library filesystem implementation for the initial browser shell.
///
/// This backend keeps and reads the file handle it opens, preventing a second
/// pathname lookup after opening. It is intentionally a partial platform
/// backend: `std::fs::File::open` cannot enforce no-follow/open-beneath
/// resolution for every path component. Production platform backends must use
/// their native containment primitive before this type is used for untrusted
/// paths.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalFileSystem;

impl FileSystem for LocalFileSystem {
    fn open(&self, path: &AuthorizedPath) -> Result<OpenedFile, FileAccessError> {
        let file = fs::File::open(path.as_path()).map_err(FileAccessError::from)?;
        if !file
            .metadata()
            .map_err(FileAccessError::from)?
            .file_type()
            .is_file()
        {
            return Err(FileAccessError::UnsupportedFile);
        }
        Ok(OpenedFile {
            file,
            media_type: None,
        })
    }
}

/// Convenience handle for sharing a filesystem implementation.
pub type SharedFileSystem = Arc<dyn FileSystem>;

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use super::*;
    use crate::file_access::FileAccessPolicy;

    static NEXT_TEMP_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    fn temporary_directory() -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "byui-browser-file-access-{}-{}",
            std::process::id(),
            NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        directory
    }

    #[test]
    fn local_filesystem_reads_regular_files_from_opened_handles() {
        let directory = temporary_directory();
        let path = directory.join("allowed.txt");
        fs::write(&path, b"authorized bytes").unwrap();
        let policy = FileAccessPolicy::new([directory.clone()]).unwrap();

        let resource = policy
            .open_file(&path, &LocalFileSystem)
            .unwrap()
            .read_all()
            .unwrap();

        assert_eq!(resource.bytes, b"authorized bytes");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn local_filesystem_rejects_directories() {
        let directory = temporary_directory();
        let policy = FileAccessPolicy::new([directory.clone()]).unwrap();
        let authorized = policy.authorize_path(&directory).unwrap();

        assert!(matches!(
            LocalFileSystem.open(&authorized),
            Err(FileAccessError::UnsupportedFile | FileAccessError::Denied)
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn opened_handle_reads_original_file_after_path_replacement() {
        let directory = temporary_directory();
        let path = directory.join("allowed.txt");
        let replacement = directory.join("replacement.txt");
        fs::write(&path, b"authorized bytes").unwrap();
        fs::write(&replacement, b"replacement bytes").unwrap();
        let policy = FileAccessPolicy::new([directory.clone()]).unwrap();

        let opened = policy.open_file(&path, &LocalFileSystem).unwrap();
        fs::rename(&replacement, &path).unwrap();

        assert_eq!(opened.read_all().unwrap().bytes, b"authorized bytes");
        fs::remove_dir_all(directory).unwrap();
    }
}
