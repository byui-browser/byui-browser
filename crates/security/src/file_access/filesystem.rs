//! Filesystem abstraction used to open files approved by security policy.

use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use cap_std::ambient_authority;
use cap_std::fs::Dir;

use super::FileAccessError;
use super::policy::AuthorizedPath;

/// Materialized data read from an authorized file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileResource {
    /// File contents in byte order.
    pub bytes: Vec<u8>,
    /// Optional media type determined by the filesystem owner.
    pub media_type: Option<String>,
}

/// An opened regular file accepted by the capability-backed filesystem.
///
/// The handle is intentionally private. Callers can materialize it with
/// [`Self::read_all`] now, while a future streaming API can retain ownership of
/// the same handle. Callers must not reopen the authorized pathname.
#[derive(Debug)]
pub struct OpenedFile {
    file: fs::File,
    media_type: Option<String>,
    length: u64,
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

    /// Reads the complete file through this opened handle.
    ///
    /// Consumes the handle, closing it after the read completes or fails. The
    /// returned bytes are therefore from the file that the backend opened,
    /// rather than from a later pathname lookup.
    pub fn read_all(mut self) -> Result<FileResource, FileAccessError> {
        let mut bytes = Vec::new();
        let mut buffer = [0; 64 * 1024];
        loop {
            match self.read_chunk(&mut buffer)? {
                0 => break,
                count => bytes.extend_from_slice(&buffer[..count]),
            }
        }
        Ok(FileResource {
            bytes,
            media_type: self.media_type,
        })
    }
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

impl CapFileSystem {
    pub(crate) fn open(&self, path: &AuthorizedPath) -> Result<OpenedFile, FileAccessError> {
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

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use super::*;
    use crate::file_access::{FileAccessPolicy, FileOrigin, FileRequestContext, FileRequestKind};

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

    fn navigation_context() -> FileRequestContext {
        FileRequestContext {
            initiating_origin: FileOrigin::Absent,
            requested_origin: FileOrigin::LocalFile,
            kind: FileRequestKind::Navigation,
        }
    }

    #[test]
    fn capability_filesystem_reads_regular_files_from_opened_handles() {
        let directory = temporary_directory();
        let path = directory.join("allowed.txt");
        fs::write(&path, b"authorized bytes").unwrap();
        let policy = FileAccessPolicy::new([directory.clone()]).unwrap();
        let filesystem = CapFileSystem::new([directory.clone()]).unwrap();

        let resource = policy
            .open_authorized_file(
                &navigation_context(),
                &policy.issue_grant(),
                &path,
                &filesystem,
            )
            .unwrap()
            .read_all()
            .unwrap();

        assert_eq!(resource.bytes, b"authorized bytes");
        drop(filesystem);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn opened_file_reports_length_and_reads_chunks_from_the_retained_handle() {
        let directory = temporary_directory();
        let path = directory.join("allowed.txt");
        fs::write(&path, b"authorized bytes").unwrap();
        let policy = FileAccessPolicy::new([directory.clone()]).unwrap();
        let filesystem = CapFileSystem::new([directory.clone()]).unwrap();

        let mut opened = policy
            .open_authorized_file(
                &navigation_context(),
                &policy.issue_grant(),
                &path,
                &filesystem,
            )
            .unwrap();
        let mut first = [0; 5];
        let mut second = [0; 32];
        let first_len = opened.read_chunk(&mut first).unwrap();
        let second_len = opened.read_chunk(&mut second).unwrap();

        assert_eq!(opened.length(), b"authorized bytes".len() as u64);
        assert_eq!(&first[..first_len], b"autho");
        assert_eq!(&second[..second_len], b"rized bytes");
        assert_eq!(opened.read_chunk(&mut second).unwrap(), 0);
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
            .open_authorized_file(
                &navigation_context(),
                &policy.issue_grant(),
                &path,
                &filesystem,
            )
            .unwrap()
            .read_all()
            .unwrap();

        assert_eq!(resource.bytes, b"authorized bytes");
        assert!(matches!(
            policy.open_authorized_file(
                &navigation_context(),
                &policy.issue_grant(),
                &outside,
                &filesystem,
            ),
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
        assert!(matches!(
            policy.open_authorized_file(
                &navigation_context(),
                &policy.issue_grant(),
                &directory,
                &filesystem,
            ),
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

        let opened = policy
            .open_authorized_file(
                &navigation_context(),
                &policy.issue_grant(),
                &path,
                &filesystem,
            )
            .unwrap();
        fs::rename(&replacement, &path).unwrap();

        assert_eq!(opened.read_all().unwrap().bytes, b"authorized bytes");
        drop(filesystem);
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn capability_filesystem_denies_a_symlink_escape() {
        use std::os::unix::fs::symlink;

        let directory = temporary_directory();
        let allowed = directory.join("allowed");
        let outside = directory.join("outside");
        let outside_file = outside.join("secret.txt");
        let link = allowed.join("escape");
        fs::create_dir(&allowed).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(&outside_file, b"denied").unwrap();
        symlink(&outside, &link).unwrap();
        let policy = FileAccessPolicy::new([allowed.clone()]).unwrap();
        let filesystem = CapFileSystem::new([allowed]).unwrap();

        assert!(matches!(
            policy.open_authorized_file(
                &navigation_context(),
                &policy.issue_grant(),
                link.join("secret.txt"),
                &filesystem
            ),
            Err(FileAccessError::Denied)
        ));
        drop(filesystem);
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn capability_filesystem_denies_a_junction_escape() {
        use std::process::Command;

        let directory = temporary_directory();
        let allowed = directory.join("allowed");
        let outside = directory.join("outside");
        let junction = allowed.join("escape");
        fs::create_dir(&allowed).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("secret.txt"), b"denied").unwrap();
        let status = Command::new("cmd.exe")
            .args([
                "/C",
                "mklink",
                "/J",
                junction.to_str().unwrap(),
                outside.to_str().unwrap(),
            ])
            .status()
            .unwrap();
        assert!(status.success(), "junction creation failed");
        let policy = FileAccessPolicy::new([allowed.clone()]).unwrap();
        let filesystem = CapFileSystem::new([allowed]).unwrap();

        assert!(matches!(
            policy.open_authorized_file(
                &navigation_context(),
                &policy.issue_grant(),
                junction.join("secret.txt"),
                &filesystem,
            ),
            Err(FileAccessError::Denied)
        ));
        drop(filesystem);
        fs::remove_dir_all(directory).unwrap();
    }
}
