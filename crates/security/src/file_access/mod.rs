//! Policy and filesystem primitives for browser-authorized `file:` access.
//!
//! This module deliberately does not depend on the `net` crate. The browser
//! layer adapts these primitives to `net::FileProvider` when composing a
//! request controller.

mod capability;
mod error;
mod handles;
mod request;

pub use capability::FileAccessCapability;
pub use error::FileAccessError;
pub use handles::{OpenedFile, TrustedDirectorySelection};
pub use request::{
    FileAccessGrant, FileAccessLease, FileOrigin, FileRequestContext, FileRequestKind,
};

#[cfg(test)]
mod tests {
    use std::{
        fs, io,
        path::{Path, PathBuf},
        sync::atomic::{AtomicUsize, Ordering},
    };

    use cap_std::{ambient_authority, fs::Dir};

    use super::capability::media_type_for_path;
    #[cfg(unix)]
    use super::capability::open_relative_nofollow;
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

    fn trusted_selection(path: &Path) -> TrustedDirectorySelection {
        let canonical_path = fs::canonicalize(path).unwrap();
        let directory = Dir::open_ambient_dir(&canonical_path, ambient_authority()).unwrap();
        TrustedDirectorySelection::from_verified_open_directory(canonical_path, directory).unwrap()
    }

    fn navigation_context() -> FileRequestContext {
        FileRequestContext {
            initiating_origin: FileOrigin::Absent,
            requested_origin: FileOrigin::LocalFile,
            kind: FileRequestKind::Navigation,
        }
    }

    #[test]
    fn trusted_selection_rejects_a_relative_selection_path() {
        let directory = temporary_directory();
        let handle = Dir::open_ambient_dir(&directory, ambient_authority()).unwrap();

        assert!(matches!(
            TrustedDirectorySelection::from_verified_open_directory(
                PathBuf::from("relative"),
                handle
            ),
            Err(FileAccessError::InvalidPath)
        ));

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn trusted_selection_rejects_a_path_and_handle_from_different_directories() {
        let directory = temporary_directory();
        let selected = directory.join("selected");
        let another = directory.join("another");
        fs::create_dir(&selected).unwrap();
        fs::create_dir(&another).unwrap();
        let selected_path = fs::canonicalize(&selected).unwrap();
        let another_handle = Dir::open_ambient_dir(&another, ambient_authority()).unwrap();

        assert!(matches!(
            TrustedDirectorySelection::from_verified_open_directory(selected_path, another_handle,),
            Err(FileAccessError::Denied)
        ));

        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn trusted_selection_rejects_root_replacement_during_configuration() {
        let directory = temporary_directory();
        let selected = directory.join("selected");
        let replacement = directory.join("replacement");
        let previous = directory.join("previous");
        fs::create_dir(&selected).unwrap();
        fs::create_dir(&replacement).unwrap();
        let canonical_path = fs::canonicalize(&selected).unwrap();
        let selected_handle = Dir::open_ambient_dir(&canonical_path, ambient_authority()).unwrap();

        fs::rename(&selected, &previous).unwrap();
        fs::rename(&replacement, &selected).unwrap();

        assert!(matches!(
            TrustedDirectorySelection::from_verified_open_directory(
                canonical_path,
                selected_handle,
            ),
            Err(FileAccessError::Denied)
        ));

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn file_access_errors_have_stable_messages_and_io_mappings() {
        for (error, message) in [
            (FileAccessError::Missing, "file is missing"),
            (FileAccessError::Denied, "file access was denied"),
            (FileAccessError::InvalidPath, "file path is invalid"),
            (FileAccessError::UnsupportedFile, "file type is unsupported"),
            (FileAccessError::Io, "file I/O failed"),
        ] {
            assert_eq!(error.to_string(), message);
        }

        assert_eq!(
            FileAccessError::from(io::Error::from(io::ErrorKind::NotFound)),
            FileAccessError::Missing
        );
        assert_eq!(
            FileAccessError::from(io::Error::from(io::ErrorKind::PermissionDenied)),
            FileAccessError::Denied
        );
        assert_eq!(
            FileAccessError::from(io::Error::from(io::ErrorKind::Other)),
            FileAccessError::Io
        );
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
            FileAccessCapability::from_trusted_directory_selections([trusted_selection(&allowed)])
                .unwrap();
        let grant = capability.issue_grant();
        let lease = grant.lease().unwrap();

        let opened = capability
            .open_authorized_file(&navigation_context(), &grant, &lease, &allowed_file)
            .unwrap();
        assert_eq!(read_all(opened), b"allowed");
        assert!(matches!(
            capability.open_authorized_file(&navigation_context(), &grant, &lease, &outside),
            Err(FileAccessError::Denied)
        ));

        drop(capability);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn outside_paths_are_denied_without_an_existence_oracle() {
        let directory = temporary_directory();
        let allowed = directory.join("allowed");
        let existing_outside = directory.join("outside.html");
        let missing_outside = directory.join("missing.html");
        fs::create_dir(&allowed).unwrap();
        fs::write(&existing_outside, b"outside").unwrap();
        let capability =
            FileAccessCapability::from_trusted_directory_selections([trusted_selection(&allowed)])
                .unwrap();
        let grant = capability.issue_grant();
        let lease = grant.lease().unwrap();

        assert!(matches!(
            capability.open_authorized_file(
                &navigation_context(),
                &grant,
                &lease,
                &existing_outside
            ),
            Err(FileAccessError::Denied)
        ));
        assert!(matches!(
            capability.open_authorized_file(
                &navigation_context(),
                &grant,
                &lease,
                &missing_outside
            ),
            Err(FileAccessError::Denied)
        ));

        drop(capability);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn missing_paths_inside_a_root_remain_missing() {
        let directory = temporary_directory();
        let capability =
            FileAccessCapability::from_trusted_directory_selections([trusted_selection(
                &directory,
            )])
            .unwrap();
        let grant = capability.issue_grant();
        let lease = grant.lease().unwrap();

        assert!(matches!(
            capability.open_authorized_file(
                &navigation_context(),
                &grant,
                &lease,
                directory.join("missing.html")
            ),
            Err(FileAccessError::Missing)
        ));

        drop(capability);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn traversal_outside_a_root_is_denied_without_an_existence_oracle() {
        let directory = temporary_directory();
        let allowed = directory.join("allowed");
        let existing_outside = directory.join("outside.html");
        fs::create_dir(&allowed).unwrap();
        fs::write(&existing_outside, b"outside").unwrap();
        let capability =
            FileAccessCapability::from_trusted_directory_selections([trusted_selection(&allowed)])
                .unwrap();
        let grant = capability.issue_grant();
        let lease = grant.lease().unwrap();

        for path in [
            allowed.join("..").join("outside.html"),
            allowed.join("..").join("missing.html"),
        ] {
            assert!(matches!(
                capability.open_authorized_file(&navigation_context(), &grant, &lease, path),
                Err(FileAccessError::Denied)
            ));
        }

        drop(capability);
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn malformed_absolute_windows_paths_are_rejected() {
        // These look like extended-length paths but omit the root directory
        // component required after the prefix. They must not enter alias
        // resolution or be interpreted relative to a trusted root.
        let capability = FileAccessCapability::from_trusted_directory_selections([]).unwrap();
        let grant = capability.issue_grant();
        let lease = grant.lease().unwrap();
        for path in [Path::new(r"\\?\C:"), Path::new(r"\\?\UNC\server")] {
            assert!(matches!(
                capability.open_authorized_file(&navigation_context(), &grant, &lease, path),
                Err(FileAccessError::InvalidPath)
            ));
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_alias_fallback_denies_existing_and_missing_paths_outside_a_root() {
        let directory = temporary_directory();
        let allowed = directory.join("allowed");
        let existing_outside = directory.join("outside.html");
        let missing_outside = directory.join("missing.html");
        fs::create_dir(&allowed).unwrap();
        fs::write(&existing_outside, b"outside").unwrap();
        let capability =
            FileAccessCapability::from_trusted_directory_selections([trusted_selection(&allowed)])
                .unwrap();

        // Exercise the filesystem-assisted spelling fallback directly. It
        // must retain the lexical path policy's non-diagnostic denial for
        // both an existing path and a missing one.
        for path in [&existing_outside, &missing_outside] {
            assert_eq!(
                capability.authorize_path(path),
                Err(FileAccessError::Denied)
            );
        }

        drop(capability);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn capability_grants_are_bound_to_one_capability() {
        let first = FileAccessCapability::from_trusted_directory_selections([]).unwrap();
        let second = FileAccessCapability::from_trusted_directory_selections([]).unwrap();

        assert_eq!(
            first.authorize_request(&navigation_context(), &second.issue_grant()),
            Err(FileAccessError::Denied)
        );
    }

    #[test]
    fn revoking_a_grant_invalidates_its_leases_and_future_open_attempts() {
        let capability = FileAccessCapability::from_trusted_directory_selections([]).unwrap();
        let grant = capability.issue_grant();
        let lease = grant.lease().unwrap();

        assert!(lease.is_current());
        grant.revoke();
        assert!(!lease.is_current());
        assert!(grant.lease().is_none());
        assert!(matches!(
            capability.open_authorized_file(
                &navigation_context(),
                &grant,
                &lease,
                Path::new("C:/not-opened"),
            ),
            Err(FileAccessError::Denied)
        ));
    }

    #[test]
    fn request_authorization_requires_a_local_navigation() {
        let capability = FileAccessCapability::from_trusted_directory_selections([]).unwrap();
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
        let capability = FileAccessCapability::from_trusted_directory_selections([]).unwrap();
        let another = FileAccessCapability::from_trusted_directory_selections([]).unwrap();
        let another_grant = another.issue_grant();
        let another_lease = another_grant.lease().unwrap();

        assert!(matches!(
            capability.open_authorized_file(
                &navigation_context(),
                &another_grant,
                &another_lease,
                "this-path-does-not-need-to-exist",
            ),
            Err(FileAccessError::Denied)
        ));
    }

    #[test]
    fn root_path_is_not_served_as_a_regular_file() {
        let directory = temporary_directory();
        let capability =
            FileAccessCapability::from_trusted_directory_selections([trusted_selection(
                &directory,
            )])
            .unwrap();
        let grant = capability.issue_grant();
        let lease = grant.lease().unwrap();

        assert!(matches!(
            capability.open_authorized_file(&navigation_context(), &grant, &lease, &directory),
            Err(FileAccessError::UnsupportedFile)
        ));

        drop(capability);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn opened_file_reports_metadata_and_reads_through_its_retained_handle() {
        let directory = temporary_directory();
        let file = directory.join("page.html");
        fs::write(&file, b"authorized bytes").unwrap();
        let capability =
            FileAccessCapability::from_trusted_directory_selections([trusted_selection(
                &directory,
            )])
            .unwrap();
        let grant = capability.issue_grant();
        let lease = grant.lease().unwrap();
        let mut opened = capability
            .open_authorized_file(&navigation_context(), &grant, &lease, &file)
            .unwrap();
        let mut first = [0; 5];
        let mut remaining = [0; 32];

        assert_eq!(opened.length(), b"authorized bytes".len() as u64);
        assert_eq!(opened.media_type(), Some("text/html; charset=utf-8"));
        let first_length = opened.read_chunk(&mut first).unwrap();
        let remaining_length = opened.read_chunk(&mut remaining).unwrap();
        assert_eq!(&first[..first_length], b"autho");
        assert_eq!(&remaining[..remaining_length], b"rized bytes");
        assert_eq!(opened.read_chunk(&mut remaining).unwrap(), 0);

        drop(opened);
        drop(capability);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn media_type_detection_is_allowlisted_and_case_insensitive() {
        for (path, media_type) in [
            ("page.HTML", Some("text/html; charset=utf-8")),
            ("stylesheet.css", Some("text/css; charset=utf-8")),
            ("module.MJS", Some("text/javascript; charset=utf-8")),
            ("document.json", Some("application/json")),
            ("image.PNG", Some("image/png")),
            ("image.JpEg", Some("image/jpeg")),
            ("image.gif", Some("image/gif")),
            ("image.SVG", Some("image/svg+xml")),
            ("image.webp", Some("image/webp")),
            ("archive.bin", None),
            ("extensionless", None),
        ] {
            assert_eq!(media_type_for_path(Path::new(path)), media_type);
        }
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
        let capability = FileAccessCapability::from_trusted_directory_selections([
            trusted_selection(&parent),
            trusted_selection(&child),
        ])
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
            FileAccessCapability::from_trusted_directory_selections([trusted_selection(&allowed)])
                .unwrap();
        let grant = capability.issue_grant();
        let lease = grant.lease().unwrap();

        assert!(matches!(
            capability.open_authorized_file(
                &navigation_context(),
                &grant,
                &lease,
                allowed.join("escape").join("secret.txt"),
            ),
            Err(FileAccessError::Denied)
        ));
        assert!(matches!(
            capability.open_authorized_file(
                &navigation_context(),
                &grant,
                &lease,
                allowed.join("escape").join("missing.txt"),
            ),
            Err(FileAccessError::Denied)
        ));

        drop(capability);
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn final_symlink_escapes_are_denied_for_existing_and_missing_targets() {
        use std::os::unix::fs::symlink;

        let directory = temporary_directory();
        let allowed = directory.join("allowed");
        let outside = directory.join("outside");
        let existing_target = outside.join("existing.html");
        fs::create_dir(&allowed).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(&existing_target, b"secret").unwrap();
        symlink(&existing_target, allowed.join("existing-link.html")).unwrap();
        symlink(
            outside.join("missing.html"),
            allowed.join("missing-link.html"),
        )
        .unwrap();
        let capability =
            FileAccessCapability::from_trusted_directory_selections([trusted_selection(&allowed)])
                .unwrap();
        let grant = capability.issue_grant();
        let lease = grant.lease().unwrap();

        for path in [
            allowed.join("existing-link.html"),
            allowed.join("missing-link.html"),
        ] {
            assert!(matches!(
                capability.open_authorized_file(&navigation_context(), &grant, &lease, path),
                Err(FileAccessError::Denied)
            ));
        }

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
            FileAccessCapability::from_trusted_directory_selections([trusted_selection(
                &directory,
            )])
            .unwrap();
        let grant = capability.issue_grant();
        let lease = grant.lease().unwrap();
        let opened = capability
            .open_authorized_file(&navigation_context(), &grant, &lease, &path)
            .unwrap();

        fs::rename(&replacement, &path).unwrap();

        assert_eq!(read_all(opened), b"original");

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
            FileAccessCapability::from_trusted_directory_selections([trusted_selection(&selected)])
                .unwrap();
        let grant = capability.issue_grant();
        let lease = grant.lease().unwrap();

        fs::rename(&selected, &previous).unwrap();
        fs::rename(&replacement, &selected).unwrap();

        let opened = capability
            .open_authorized_file(
                &navigation_context(),
                &grant,
                &lease,
                selected.join("page.html"),
            )
            .unwrap();
        assert_eq!(read_all(opened), b"selected root");

        drop(capability);
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn retained_root_open_rejects_a_final_symlink_component() {
        use std::os::unix::fs::symlink;

        let directory = temporary_directory();
        let selected = directory.join("selected");
        let target = selected.join("target.txt");
        let link = selected.join("link.txt");
        fs::create_dir(&selected).unwrap();
        fs::write(&target, b"target").unwrap();
        symlink(&target, &link).unwrap();
        let root = trusted_selection(&selected);

        assert!(open_relative_nofollow(&root.directory, Path::new("link.txt")).is_err());

        drop(root);
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn retained_root_open_rejects_an_intermediate_symlink_component() {
        use std::os::unix::fs::symlink;

        let directory = temporary_directory();
        let selected = directory.join("selected");
        let target = selected.join("target");
        let link = selected.join("link");
        fs::create_dir(&selected).unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(target.join("page.txt"), b"target").unwrap();
        symlink(&target, &link).unwrap();
        let root = trusted_selection(&selected);

        assert!(open_relative_nofollow(&root.directory, Path::new("link/page.txt")).is_err());

        drop(root);
        fs::remove_dir_all(directory).unwrap();
    }

    fn read_all(mut opened: OpenedFile) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut buffer = [0; 64 * 1024];
        loop {
            match opened.read_chunk(&mut buffer).unwrap() {
                0 => return bytes,
                count => bytes.extend_from_slice(&buffer[..count]),
            }
        }
    }
}
