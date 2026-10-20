use std::{
    fs,
    sync::Arc,
    sync::atomic::{AtomicUsize, Ordering},
};

use browser::{FileAccessConfiguration, file_provider::FileAccessGrant, request_controller};
use security::file_access::{CapFileSystem, FileAccessPermission, FileAccessPolicy, FileSandboxId};

static NEXT_TEMP_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

fn temporary_directory() -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "byui-browser-provider-{}-{}",
        std::process::id(),
        NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&directory).unwrap();
    directory
}

fn file_access(
    policy: Arc<FileAccessPolicy>,
    filesystem: Arc<CapFileSystem>,
    permission: FileAccessPermission,
) -> FileAccessConfiguration {
    FileAccessConfiguration::new(
        policy,
        filesystem,
        FileAccessGrant::new(permission, FileSandboxId::default()),
    )
}

fn navigation(url: impl Into<String>) -> net::Request {
    let mut request = net::Request::get(url);
    request.set_mode(net::RequestMode::Navigate).unwrap();
    request
}

#[test]
fn configured_file_provider_reads_allowed_files_and_keeps_default_deny() {
    let directory = temporary_directory();
    let allowed = directory.join("allowed");
    let file = allowed.join("page.html");
    let outside = directory.join("outside.html");
    fs::create_dir(&allowed).unwrap();
    fs::write(&file, b"<h1>allowed</h1>").unwrap();
    fs::write(&outside, b"<h1>denied</h1>").unwrap();

    let policy = Arc::new(FileAccessPolicy::new([allowed.clone()]).unwrap());
    let filesystem = Arc::new(CapFileSystem::new([allowed.clone()]).unwrap());
    let controller = request_controller(Some(file_access(
        policy,
        filesystem,
        FileAccessPermission::Granted,
    )));
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let allowed_url = net::Url::from_file_path(&file).unwrap();
    let outside_url = net::Url::from_file_path(&outside).unwrap();
    let response = runtime
        .block_on(controller.fetch(navigation(allowed_url.clone())))
        .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.headers.get("content-length"), Some("16".into()));
    assert_eq!(
        runtime.block_on(response.body.bytes()).unwrap(),
        b"<h1>allowed</h1>"
    );

    let denied = runtime.block_on(controller.fetch(navigation(outside_url)));
    assert!(matches!(
        denied,
        Err(net::RequestError::Provider {
            provider: "file",
            kind: net::ProviderErrorKind::AccessDenied
        })
    ));

    let head = runtime
        .block_on(controller.fetch({
            let mut request = navigation(allowed_url);
            request.set_method(net::Method::HEAD).unwrap();
            request
        }))
        .unwrap();
    assert!(head.body.is_null());

    drop(head);
    drop(controller);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn request_controller_without_file_configuration_has_no_file_capability() {
    let controller = request_controller(None);
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let result = runtime.block_on(controller.fetch(net::Request::get("file:///missing")));
    assert!(matches!(
        result,
        Err(net::RequestError::MissingProvider("file"))
    ));
}

#[test]
fn file_provider_maps_missing_directories_and_malformed_urls() {
    let directory = temporary_directory();
    let allowed = directory.join("allowed");
    let file = allowed.join("page.html");
    fs::create_dir(&allowed).unwrap();
    fs::write(&file, b"allowed").unwrap();
    let policy = Arc::new(FileAccessPolicy::new([allowed.clone()]).unwrap());
    let filesystem = Arc::new(CapFileSystem::new([allowed.clone()]).unwrap());
    let controller = request_controller(Some(file_access(
        policy,
        filesystem,
        FileAccessPermission::Granted,
    )));
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let missing = net::Url::from_file_path(directory.join("missing.html")).unwrap();
    assert!(matches!(
        runtime.block_on(controller.fetch(navigation(missing))),
        Err(net::RequestError::Provider {
            provider: "file",
            kind: net::ProviderErrorKind::ResourceMissing
        })
    ));

    let directory_url = net::Url::from_file_path(&allowed).unwrap();
    assert!(matches!(
        runtime.block_on(controller.fetch(navigation(directory_url))),
        Err(net::RequestError::Provider {
            provider: "file",
            kind: net::ProviderErrorKind::Io
        })
    ));

    let malformed = net::Url::parse("file:///allowed/%00").unwrap();
    assert!(matches!(
        runtime.block_on(controller.fetch(navigation(malformed))),
        Err(net::RequestError::Provider {
            provider: "file",
            kind: net::ProviderErrorKind::MalformedMetadata
        })
    ));

    drop(controller);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn file_provider_denies_subresources_permissions_and_sandbox_mismatches_before_opening() {
    let directory = temporary_directory();
    let file = directory.join("page.html");
    fs::write(&file, b"must not be exposed").unwrap();
    let policy = Arc::new(
        FileAccessPolicy::new_in_sandbox([directory.clone()], FileSandboxId::new("profile-a"))
            .unwrap(),
    );
    let filesystem = Arc::new(CapFileSystem::new([directory.clone()]).unwrap());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let url = net::Url::from_file_path(&file).unwrap();

    let denied_permission = request_controller(Some(FileAccessConfiguration::new(
        policy.clone(),
        filesystem.clone(),
        FileAccessGrant::new(
            FileAccessPermission::Denied,
            FileSandboxId::new("profile-a"),
        ),
    )));
    assert!(matches!(
        runtime.block_on(denied_permission.fetch(navigation(url.clone()))),
        Err(net::RequestError::Provider {
            provider: "file",
            kind: net::ProviderErrorKind::AccessDenied
        })
    ));

    let mismatched_sandbox = request_controller(Some(FileAccessConfiguration::new(
        policy.clone(),
        filesystem.clone(),
        FileAccessGrant::new(
            FileAccessPermission::Granted,
            FileSandboxId::new("profile-b"),
        ),
    )));
    assert!(matches!(
        runtime.block_on(mismatched_sandbox.fetch(navigation(url.clone()))),
        Err(net::RequestError::Provider {
            provider: "file",
            kind: net::ProviderErrorKind::AccessDenied
        })
    ));

    let subresource = request_controller(Some(FileAccessConfiguration::new(
        policy,
        filesystem,
        FileAccessGrant::new(
            FileAccessPermission::Granted,
            FileSandboxId::new("profile-a"),
        ),
    )));
    assert!(matches!(
        runtime.block_on(subresource.fetch(net::Request::get(url))),
        Err(net::RequestError::Provider {
            provider: "file",
            kind: net::ProviderErrorKind::AccessDenied
        })
    ));

    drop(subresource);
    drop(mismatched_sandbox);
    drop(denied_permission);
    fs::remove_dir_all(directory).unwrap();
}
