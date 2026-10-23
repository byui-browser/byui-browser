use std::{
    fs,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

use browser::{FileAccessConfiguration, request_controller};
use cap_std::{ambient_authority, fs::Dir};
use futures_util::StreamExt;
use net::AbortController;
use security::file_access::TrustedFileAccessRoot;

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
    allowed_roots: impl IntoIterator<Item = std::path::PathBuf>,
) -> FileAccessConfiguration {
    let roots = allowed_roots
        .into_iter()
        .map(|path| {
            let canonical_path = fs::canonicalize(&path).unwrap();
            let directory = Dir::open_ambient_dir(&canonical_path, ambient_authority()).unwrap();
            TrustedFileAccessRoot::from_open_directory(canonical_path, directory).unwrap()
        })
        .collect::<Vec<_>>();
    FileAccessConfiguration::from_trusted_roots(roots).unwrap()
}

fn navigation(file_access: &FileAccessConfiguration, url: impl Into<String>) -> net::Request {
    file_access.navigation_request(url).unwrap()
}

fn navigation_with_client_origin(
    file_access: &FileAccessConfiguration,
    url: impl Into<String>,
) -> net::Request {
    let mut request = navigation(file_access, url);
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

    let file_access = file_access([allowed.clone()]);
    let controller = request_controller(Some(file_access.clone()));
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let allowed_url = net::Url::from_file_path(&file).unwrap();
    let outside_url = net::Url::from_file_path(&outside).unwrap();
    let response = runtime
        .block_on(controller.fetch(navigation(&file_access, allowed_url.clone())))
        .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.headers.get("content-length"), Some("16".into()));
    assert_eq!(
        runtime.block_on(response.body.bytes()).unwrap(),
        b"<h1>allowed</h1>"
    );

    let denied = runtime.block_on(controller.fetch(navigation(&file_access, outside_url)));
    assert!(matches!(
        denied,
        Err(net::RequestError::Provider {
            provider: "file",
            kind: net::ProviderErrorKind::AccessDenied
        })
    ));

    let head = runtime
        .block_on(controller.fetch({
            let mut request = navigation(&file_access, allowed_url);
            request.set_method(net::Method::HEAD).unwrap();
            request
        }))
        .unwrap();
    assert!(head.body.is_null());

    drop(head);
    drop(controller);
    drop(file_access);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn configured_file_provider_accepts_navigation_with_a_client_origin() {
    let directory = temporary_directory();
    let allowed = directory.join("allowed");
    let file = allowed.join("page.html");
    fs::create_dir(&allowed).unwrap();
    fs::write(&file, b"context-aware").unwrap();
    let file_access = file_access([allowed]);
    let controller = request_controller(Some(file_access.clone()));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let url = net::Url::from_file_path(&file).unwrap();

    let response = runtime
        .block_on(controller.fetch(navigation_with_client_origin(&file_access, url)))
        .unwrap();
    assert_eq!(
        runtime.block_on(response.body.bytes()).unwrap(),
        b"context-aware"
    );

    drop(controller);
    drop(file_access);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn revoking_a_file_access_grant_denies_subsequent_opens() {
    let directory = temporary_directory();
    let allowed = directory.join("allowed");
    let file = allowed.join("page.html");
    fs::create_dir(&allowed).unwrap();
    fs::write(&file, b"revocable").unwrap();
    let file_access = file_access([allowed]);
    let grants = file_access.grant_source();
    let navigation_access = file_access.clone();
    let controller = request_controller(Some(file_access));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let url = net::Url::from_file_path(&file).unwrap();

    let response = runtime
        .block_on(controller.fetch(navigation(&navigation_access, url.clone())))
        .unwrap();
    assert_eq!(
        runtime.block_on(response.body.bytes()).unwrap(),
        b"revocable"
    );

    grants.revoke();
    assert!(matches!(
        navigation_access.navigation_request(url.to_string()),
        Err(browser::file_provider::FileNavigationError::SessionRevoked)
    ));

    drop(controller);
    drop(navigation_access);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn closing_a_file_access_session_cancels_live_file_bodies() {
    let directory = temporary_directory();
    let allowed = directory.join("allowed");
    let file = allowed.join("large-page.html");
    fs::create_dir(&allowed).unwrap();
    fs::write(&file, vec![b'x'; 4 * 1024 * 1024]).unwrap();
    let file_access = file_access([allowed]);
    let navigation_access = file_access.clone();
    let controller = request_controller(Some(file_access));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let url = net::Url::from_file_path(&file).unwrap();

    runtime.block_on(async {
        let response = controller
            .fetch(navigation(&navigation_access, url.clone()))
            .await
            .unwrap();
        navigation_access.navigation_away();

        let error = response.body.into_future().await.0.unwrap().unwrap_err();
        assert!(matches!(error, net::RequestError::Aborted));
    });
    assert!(matches!(
        navigation_access.navigation_request(url.to_string()),
        Err(browser::file_provider::FileNavigationError::SessionRevoked)
    ));

    drop(controller);
    drop(navigation_access);
    drop(runtime);
    for _ in 0..100 {
        if fs::remove_dir_all(&directory).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("closed file session should release its opened handle");
}

#[test]
fn configured_file_provider_returns_a_live_chunked_body() {
    let directory = temporary_directory();
    let allowed = directory.join("allowed");
    let file = allowed.join("large-page.html");
    let expected = vec![b'x'; 192 * 1024];
    fs::create_dir(&allowed).unwrap();
    fs::write(&file, &expected).unwrap();

    let file_access = file_access([allowed]);
    let controller = request_controller(Some(file_access.clone()));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let url = net::Url::from_file_path(&file).unwrap();

    runtime.block_on(async {
        let response = controller
            .fetch(navigation(&file_access, url))
            .await
            .unwrap();
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
    drop(file_access);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn aborting_a_live_file_body_stops_the_blocking_reader() {
    let directory = temporary_directory();
    let allowed = directory.join("allowed");
    let file = allowed.join("large-page.html");
    fs::create_dir(&allowed).unwrap();
    fs::write(&file, vec![b'x'; 4 * 1024 * 1024]).unwrap();

    let file_access = file_access([allowed]);
    let controller = request_controller(Some(file_access.clone()));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let url = net::Url::from_file_path(&file).unwrap();

    runtime.block_on(async {
        let abort = AbortController::new();
        let mut request = navigation(&file_access, url);
        request.set_signal(abort.signal());
        let response = controller.fetch(request).await.unwrap();
        abort.abort();

        let error = response.body.into_future().await.0.unwrap().unwrap_err();
        assert!(matches!(error, net::RequestError::Aborted));
    });

    drop(controller);
    drop(file_access);
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
    let file_access = file_access([allowed.clone()]);
    let controller = request_controller(Some(file_access.clone()));
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let missing = net::Url::from_file_path(directory.join("missing.html")).unwrap();
    assert!(matches!(
        runtime.block_on(controller.fetch(navigation(&file_access, missing))),
        Err(net::RequestError::Provider {
            provider: "file",
            kind: net::ProviderErrorKind::ResourceMissing
        })
    ));

    let directory_url = net::Url::from_file_path(&allowed).unwrap();
    assert!(matches!(
        runtime.block_on(controller.fetch(navigation(&file_access, directory_url))),
        Err(net::RequestError::Provider {
            provider: "file",
            kind: net::ProviderErrorKind::Io
        })
    ));

    let malformed = net::Url::parse("file:///allowed/%00").unwrap();
    assert!(matches!(
        runtime.block_on(controller.fetch(navigation(&file_access, malformed))),
        Err(net::RequestError::Provider {
            provider: "file",
            kind: net::ProviderErrorKind::MalformedMetadata
        })
    ));

    drop(controller);
    drop(file_access);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn file_provider_denies_revoked_and_subresource_requests_before_opening() {
    let directory = temporary_directory();
    let file = directory.join("page.html");
    fs::write(&file, b"must not be exposed").unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let url = net::Url::from_file_path(&file).unwrap();
    let missing_url = net::Url::from_file_path(directory.join("missing.html")).unwrap();

    let revoked_access = file_access([directory.clone()]);
    let revoked_request = navigation(&revoked_access, missing_url);
    let revoked_grants = revoked_access.grant_source();
    revoked_grants.revoke();
    let revoked = request_controller(Some(revoked_access));
    assert!(matches!(
        runtime.block_on(revoked.fetch(revoked_request)),
        Err(net::RequestError::Provider {
            provider: "file",
            kind: net::ProviderErrorKind::AccessDenied
        })
    ));

    let subresource = request_controller(Some(file_access([directory.clone()])));
    assert!(matches!(
        runtime.block_on(subresource.fetch(net::Request::get(url))),
        Err(net::RequestError::Provider {
            provider: "file",
            kind: net::ProviderErrorKind::AccessDenied
        })
    ));

    drop(subresource);
    drop(revoked);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn file_navigation_requires_a_browser_issued_single_use_authorization() {
    let directory = temporary_directory();
    let file = directory.join("page.html");
    fs::write(&file, b"authorized").unwrap();
    let file_access = file_access([directory.clone()]);
    let controller = request_controller(Some(file_access.clone()));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let url = net::Url::from_file_path(&file).unwrap();

    let mut caller_controlled = net::Request::get(url.clone());
    caller_controlled
        .set_mode(net::RequestMode::Navigate)
        .unwrap();
    assert!(matches!(
        runtime.block_on(controller.fetch(caller_controlled)),
        Err(net::RequestError::Provider {
            provider: "file",
            kind: net::ProviderErrorKind::AccessDenied
        })
    ));

    let mut malformed = net::Request::get("file:///allowed/%00");
    malformed.set_mode(net::RequestMode::Navigate).unwrap();
    assert!(matches!(
        runtime.block_on(controller.fetch(malformed)),
        Err(net::RequestError::Provider {
            provider: "file",
            kind: net::ProviderErrorKind::AccessDenied
        })
    ));

    let authorized = navigation(&file_access, url);
    let response = runtime
        .block_on(controller.fetch(authorized.clone()))
        .unwrap();
    drop(response);
    assert!(matches!(
        runtime.block_on(controller.fetch(authorized)),
        Err(net::RequestError::Provider {
            provider: "file",
            kind: net::ProviderErrorKind::AccessDenied
        })
    ));

    drop(controller);
    drop(file_access);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn pending_file_navigation_authorizations_are_bounded() {
    let directory = temporary_directory();
    let file_access = file_access([directory.clone()]);

    for _ in 0..64 {
        file_access.navigation_request("file:///pending").unwrap();
    }
    assert!(matches!(
        file_access.navigation_request("file:///overflow"),
        Err(browser::file_provider::FileNavigationError::PendingLimitReached { limit: 64 })
    ));

    drop(file_access);
    fs::remove_dir_all(directory).unwrap();
}
