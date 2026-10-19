//! Filesystem abstraction used to open files approved by security policy.

use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

use cap_std::ambient_authority;
use cap_std::fs::Dir;

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

/// Capability-oriented filesystem backend for browser-authorized files.
///
/// Each configured root is opened as a directory capability. Requests are
/// then resolved relative to that capability, so path components cannot
/// escape through a junction or symlink race. The roots passed here must be
/// the same configured roots used to construct the corresponding
/// [`FileAccessPolicy`].
#[derive(Debug)]
pub struct CapFileSystem {
    roots: Vec<(PathBuf, Dir)>,
}

impl CapFileSystem {
    /// Opens directory capabilities for the configured allowed roots.
    pub fn new(allowed_roots: impl IntoIterator<Item = PathBuf>) -> Result<Self, FileAccessError> {
        let roots = allowed_roots
            .into_iter()
            .map(|root| {
                let canonical = fs::canonicalize(root).map_err(FileAccessError::from)?;
                let directory = Dir::open_ambient_dir(&canonical, ambient_authority())
                    .map_err(FileAccessError::from)?;
                Ok((canonical, directory))
            })
            .collect::<Result<Vec<_>, FileAccessError>>()?;
        Ok(Self { roots })
    }
}

impl FileSystem for CapFileSystem {
    fn open(&self, path: &AuthorizedPath) -> Result<OpenedFile, FileAccessError> {
        if path.relative() == Path::new(".") {
            return Err(FileAccessError::UnsupportedFile);
        }
        let (_, root) = self
            .roots
            .iter()
            .find(|(configured_root, _)| configured_root == path.root())
            .ok_or(FileAccessError::Denied)?;
        let file = root.open(path.relative()).map_err(FileAccessError::from)?;
        let file = file.into_std();
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
    fn capability_filesystem_reads_regular_files_from_opened_handles() {
        let directory = temporary_directory();
        let path = directory.join("allowed.txt");
        fs::write(&path, b"authorized bytes").unwrap();
        let policy = FileAccessPolicy::new([directory.clone()]).unwrap();
        let filesystem = CapFileSystem::new([directory.clone()]).unwrap();

        let resource = policy
            .open_file(&path, &filesystem)
            .unwrap()
            .read_all()
            .unwrap();

        assert_eq!(resource.bytes, b"authorized bytes");
        drop(filesystem);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn capability_filesystem_reads_only_from_the_configured_root() {
        let directory = temporary_directory();
        let allowed = directory.join("allowed");
        let outside = directory.join("outside.txt");
        let path = allowed.join("nested.txt");
        fs::create_dir(&allowed).unwrap();
        fs::write(&path, b"authorized bytes").unwrap();
        fs::write(&outside, b"denied bytes").unwrap();
        let policy = FileAccessPolicy::new([allowed.clone()]).unwrap();
        let filesystem = CapFileSystem::new([allowed.clone()]).unwrap();

        let resource = policy
            .open_file(&path, &filesystem)
            .unwrap()
            .read_all()
            .unwrap();

        assert_eq!(resource.bytes, b"authorized bytes");
        assert!(matches!(
            policy.open_file(&outside, &filesystem),
            Err(FileAccessError::Denied)
        ));
        drop(filesystem);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn capability_filesystem_rejects_directories() {
        let directory = temporary_directory();
        let policy = FileAccessPolicy::new([directory.clone()]).unwrap();
        let filesystem = CapFileSystem::new([directory.clone()]).unwrap();
        let authorized = policy.authorize_path(&directory).unwrap();

        assert!(matches!(
            filesystem.open(&authorized),
            Err(FileAccessError::UnsupportedFile)
        ));
        drop(filesystem);
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn capability_filesystem_reads_the_opened_file_after_path_replacement() {
        let directory = temporary_directory();
        let path = directory.join("allowed.txt");
        let replacement = directory.join("replacement.txt");
        fs::write(&path, b"authorized bytes").unwrap();
        fs::write(&replacement, b"replacement bytes").unwrap();
        let policy = FileAccessPolicy::new([directory.clone()]).unwrap();
        let filesystem = CapFileSystem::new([directory.clone()]).unwrap();

        let opened = policy.open_file(&path, &filesystem).unwrap();
        fs::rename(&replacement, &path).unwrap();

        assert_eq!(opened.read_all().unwrap().bytes, b"authorized bytes");
        drop(filesystem);
        fs::remove_dir_all(directory).unwrap();
    }
}
