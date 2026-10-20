use std::{
    fs,
    sync::Arc,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

use browser::{FileAccessConfiguration, file_provider::FileAccessGrantSource, request_controller};
use futures_util::StreamExt;
use net::AbortController;
use security::file_access::{CapFileSystem, FileAccessPolicy};

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
) -> FileAccessConfiguration {
    let grants = FileAccessGrantSource::new(policy.issue_grant());
    FileAccessConfiguration::with_grant_source(policy, filesystem, grants)
}

fn navigation(url: impl Into<String>) -> net::Request {
    let mut request = net::Request::get(url);
    request.set_mode(net::RequestMode::Navigate).unwrap();
    request
}

fn navigation_with_client_origin(url: impl Into<String>) -> net::Request {
    let mut request = navigation(url);
    request.set_environment(
        net::FetchEnvironment::from_url(net::Url::parse("https://client.test/page").unwrap())
            .unwrap(),
    );
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
    let controller = request_controller(Some(file_access(policy, filesystem)));
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
fn configured_file_provider_accepts_navigation_with_a_client_origin() {
    let directory = temporary_directory();
    let allowed = directory.join("allowed");
    let file = allowed.join("page.html");
    fs::create_dir(&allowed).unwrap();
    fs::write(&file, b"context-aware").unwrap();
    let policy = Arc::new(FileAccessPolicy::new([allowed.clone()]).unwrap());
    let filesystem = Arc::new(CapFileSystem::new([allowed]).unwrap());
    let controller = request_controller(Some(file_access(policy, filesystem)));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let url = net::Url::from_file_path(&file).unwrap();

    let response = runtime
        .block_on(controller.fetch(navigation_with_client_origin(url)))
        .unwrap();
    assert_eq!(
        runtime.block_on(response.body.bytes()).unwrap(),
        b"context-aware"
    );

    drop(controller);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn revoking_a_file_access_grant_denies_subsequent_opens() {
    let directory = temporary_directory();
    let allowed = directory.join("allowed");
    let file = allowed.join("page.html");
    fs::create_dir(&allowed).unwrap();
    fs::write(&file, b"revocable").unwrap();
    let policy = Arc::new(FileAccessPolicy::new([allowed.clone()]).unwrap());
    let filesystem = Arc::new(CapFileSystem::new([allowed]).unwrap());
    let grants = FileAccessGrantSource::new(policy.issue_grant());
    let controller = request_controller(Some(FileAccessConfiguration::with_grant_source(
        policy,
        filesystem,
        grants.clone(),
    )));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let url = net::Url::from_file_path(&file).unwrap();

    let response = runtime
        .block_on(controller.fetch(navigation(url.clone())))
        .unwrap();
    assert_eq!(
        runtime.block_on(response.body.bytes()).unwrap(),
        b"revocable"
    );

    grants.revoke();
    assert!(matches!(
        runtime.block_on(controller.fetch(navigation(url))),
        Err(net::RequestError::Provider {
            provider: "file",
            kind: net::ProviderErrorKind::AccessDenied
        })
    ));

    drop(controller);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn configured_file_provider_returns_a_live_chunked_body() {
    let directory = temporary_directory();
    let allowed = directory.join("allowed");
    let file = allowed.join("large-page.html");
    let expected = vec![b'x'; 192 * 1024];
    fs::create_dir(&allowed).unwrap();
    fs::write(&file, &expected).unwrap();

    let policy = Arc::new(FileAccessPolicy::new([allowed.clone()]).unwrap());
    let filesystem = Arc::new(CapFileSystem::new([allowed]).unwrap());
    let controller = request_controller(Some(file_access(policy, filesystem)));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let url = net::Url::from_file_path(&file).unwrap();

    runtime.block_on(async {
        let response = controller.fetch(navigation(url)).await.unwrap();
        assert_eq!(
            response.headers.get("content-length"),
            Some(expected.len().to_string())
        );
        assert!(!response.body.is_replayable());

        let mut body = response.body;
        let first = body.next().await.unwrap().unwrap();
        assert!(!first.is_empty());
        let mut received = first.to_vec();
        while let Some(chunk) = body.next().await {
            received.extend_from_slice(&chunk.unwrap());
        }
        assert_eq!(received, expected);
    });

    drop(controller);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn aborting_a_live_file_body_stops_the_blocking_reader() {
    let directory = temporary_directory();
    let allowed = directory.join("allowed");
    let file = allowed.join("large-page.html");
    fs::create_dir(&allowed).unwrap();
    fs::write(&file, vec![b'x'; 4 * 1024 * 1024]).unwrap();

    let policy = Arc::new(FileAccessPolicy::new([allowed.clone()]).unwrap());
    let filesystem = Arc::new(CapFileSystem::new([allowed]).unwrap());
    let controller = request_controller(Some(file_access(policy, filesystem)));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let url = net::Url::from_file_path(&file).unwrap();

    runtime.block_on(async {
        let abort = AbortController::new();
        let mut request = navigation(url);
        request.set_signal(abort.signal());
        let response = controller.fetch(request).await.unwrap();
        abort.abort();

        let error = response.body.into_future().await.0.unwrap().unwrap_err();
        assert!(matches!(error, net::RequestError::Aborted));
    });

    drop(controller);
    drop(runtime);
    for _ in 0..100 {
        if fs::remove_dir_all(&directory).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("aborted file reader should release its opened handle");
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
    let controller = request_controller(Some(file_access(policy, filesystem)));
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
fn file_provider_denies_revoked_foreign_and_subresource_requests_before_opening() {
    let directory = temporary_directory();
    let file = directory.join("page.html");
    fs::write(&file, b"must not be exposed").unwrap();
    let policy = Arc::new(FileAccessPolicy::new([directory.clone()]).unwrap());
    let foreign_policy = FileAccessPolicy::new([directory.clone()]).unwrap();
    let filesystem = Arc::new(CapFileSystem::new([directory.clone()]).unwrap());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let url = net::Url::from_file_path(&file).unwrap();
    let missing_url = net::Url::from_file_path(directory.join("missing.html")).unwrap();

    let revoked_grants = FileAccessGrantSource::new(policy.issue_grant());
    revoked_grants.revoke();
    let revoked = request_controller(Some(FileAccessConfiguration::with_grant_source(
        policy.clone(),
        filesystem.clone(),
        revoked_grants,
    )));
    assert!(matches!(
        runtime.block_on(revoked.fetch(navigation(missing_url))),
        Err(net::RequestError::Provider {
            provider: "file",
            kind: net::ProviderErrorKind::AccessDenied
        })
    ));

    let foreign_grant = request_controller(Some(FileAccessConfiguration::with_grant_source(
        policy.clone(),
        filesystem.clone(),
        FileAccessGrantSource::new(foreign_policy.issue_grant()),
    )));
    assert!(matches!(
        runtime.block_on(foreign_grant.fetch(navigation(url.clone()))),
        Err(net::RequestError::Provider {
            provider: "file",
            kind: net::ProviderErrorKind::AccessDenied
        })
    ));

    let grant = policy.issue_grant();
    let subresource = request_controller(Some(FileAccessConfiguration::with_grant_source(
        policy,
        filesystem,
        FileAccessGrantSource::new(grant),
    )));
    assert!(matches!(
        runtime.block_on(subresource.fetch(net::Request::get(url))),
        Err(net::RequestError::Provider {
            provider: "file",
            kind: net::ProviderErrorKind::AccessDenied
        })
    ));

    drop(subresource);
    drop(foreign_grant);
    drop(revoked);
    fs::remove_dir_all(directory).unwrap();
}
