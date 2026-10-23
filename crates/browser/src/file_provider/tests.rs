use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use cap_std::{ambient_authority, fs::Dir};
use futures_util::StreamExt;
use net::{
    AbortSignal, FileProvider, HeaderValue, ProviderErrorKind, Request, RequestError, SchemeBody,
    Url,
};
use security::file_access::{FileAccessCapability, TrustedDirectorySelection};
use tokio::sync::mpsc;

use super::{
    FileAccessGrantSource, FileNavigationAuthorizations,
    admission::{FileReadAdmission, MAX_CONCURRENT_FILE_READS, MAX_PENDING_FILE_READS},
    provider::BrowserFileProvider,
    reader::FILE_READ_CHUNK_SIZE,
    session::FileAccessCancellation,
    stream::FileBodyStream,
};

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

#[test]
fn file_read_admission_rejects_excess_waiters_and_cancels_queued_work() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let admission = FileReadAdmission::new(0, 1);
        let abort = net::AbortController::new();
        let session_cancellation = FileAccessCancellation::new();
        let waiting_admission = admission.clone();
        let waiting_signal = abort.signal();
        let waiting_session_cancellation = session_cancellation.clone();
        let waiting = tokio::spawn(async move {
            waiting_admission
                .acquire(&waiting_signal, &waiting_session_cancellation)
                .await
        });

        while admission.pending.available_permits() != 0 {
            tokio::task::yield_now().await;
        }
        assert!(matches!(
            admission
                .acquire(&AbortSignal::new(), &session_cancellation)
                .await,
            Err(RequestError::Provider {
                provider: "file",
                kind: ProviderErrorKind::ResourceExhausted,
            })
        ));

        abort.abort();
        assert!(matches!(waiting.await.unwrap(), Err(RequestError::Aborted)));

        let session_admission = FileReadAdmission::new(0, 1);
        let session_cancellation = FileAccessCancellation::new();
        let waiting_admission = session_admission.clone();
        let waiting_cancellation = session_cancellation.clone();
        let session_waiting = tokio::spawn(async move {
            waiting_admission
                .acquire(&AbortSignal::new(), &waiting_cancellation)
                .await
        });
        while session_admission.pending.available_permits() != 0 {
            tokio::task::yield_now().await;
        }
        session_cancellation.cancel();
        assert!(matches!(
            session_waiting.await.unwrap(),
            Err(RequestError::Aborted)
        ));
    });
}

#[test]
fn revocation_after_file_setup_rejects_get_and_head_and_releases_admission() {
    let directory = std::env::temp_dir().join(format!(
        "byui-browser-post-setup-abort-{}-{}",
        std::process::id(),
        NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&directory).unwrap();
    let file = directory.join("page.html");
    fs::write(&file, b"prepared content").unwrap();
    let canonical_directory = fs::canonicalize(&directory).unwrap();
    let selection = TrustedDirectorySelection::from_verified_open_directory(
        canonical_directory,
        Dir::open_ambient_dir(&directory, ambient_authority()).unwrap(),
    )
    .unwrap();
    let capability =
        Arc::new(FileAccessCapability::from_trusted_directory_selections([selection]).unwrap());
    let navigation_authorizations = Arc::new(FileNavigationAuthorizations::new());
    let grants = FileAccessGrantSource::new(
        capability.issue_grant(),
        Arc::clone(&navigation_authorizations),
    );
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let url = Url::from_file_path(&file).unwrap();

    for method in [net::Method::GET, net::Method::HEAD] {
        let hook_grants = grants.clone();
        let mut provider = BrowserFileProvider::new(
            Arc::clone(&capability),
            grants.clone(),
            Arc::clone(&navigation_authorizations),
            u64::MAX,
        );
        provider.post_setup_hook = Some(Arc::new(move || hook_grants.revoke()));
        let mut request = Request::get(url.clone());
        request.set_method(method).unwrap();
        request.set_mode(net::RequestMode::Navigate).unwrap();
        navigation_authorizations.authorize(request.id()).unwrap();

        assert!(matches!(
            runtime.block_on(provider.open(&request, &url)),
            Err(RequestError::Aborted)
        ));
        assert_eq!(
            provider.read_admission.active.available_permits(),
            MAX_CONCURRENT_FILE_READS
        );
        assert_eq!(
            provider.read_admission.pending.available_permits(),
            MAX_PENDING_FILE_READS
        );

        grants.replace(capability.issue_grant());
        let probe = BrowserFileProvider {
            capability: Arc::clone(&provider.capability),
            grants: provider.grants.clone(),
            navigation_authorizations: Arc::clone(&provider.navigation_authorizations),
            read_admission: provider.read_admission.clone(),
            max_file_size: provider.max_file_size,
            post_setup_hook: None,
        };
        let mut probe_request = Request::get(url.clone());
        probe_request.set_method(net::Method::HEAD).unwrap();
        probe_request.set_mode(net::RequestMode::Navigate).unwrap();
        navigation_authorizations
            .authorize(probe_request.id())
            .unwrap();
        let response = runtime.block_on(probe.open(&probe_request, &url)).unwrap();
        assert!(matches!(response.body, SchemeBody::Null));
    }

    drop(runtime);
    drop(capability);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn revoked_session_never_delivers_an_already_queued_chunk() {
    let capability = FileAccessCapability::from_trusted_directory_selections([]).unwrap();
    let grant = capability.issue_grant();
    let lease = grant.lease().unwrap();
    let (sender, receiver) = mpsc::channel(1);
    sender.try_send(Ok(vec![b'x'])).unwrap();
    grant.revoke();
    let mut body = FileBodyStream {
        receiver,
        cancelled: Arc::new(AtomicBool::new(false)),
        session_cancellation: FileAccessCancellation::new(),
        lease,
        reported_session_cancellation: false,
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();

    assert!(matches!(
        runtime.block_on(body.next()),
        Some(Err(RequestError::Aborted))
    ));
}

#[test]
fn dropping_unread_or_partially_read_body_releases_reader_admission() {
    let directory = std::env::temp_dir().join(format!(
        "byui-browser-body-drop-{}-{}",
        std::process::id(),
        NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&directory).unwrap();
    let file = directory.join("page.html");
    fs::write(&file, vec![b'x'; FILE_READ_CHUNK_SIZE * 4]).unwrap();
    let canonical_directory = fs::canonicalize(&directory).unwrap();
    let selection = TrustedDirectorySelection::from_verified_open_directory(
        canonical_directory,
        Dir::open_ambient_dir(&directory, ambient_authority()).unwrap(),
    )
    .unwrap();
    let capability =
        Arc::new(FileAccessCapability::from_trusted_directory_selections([selection]).unwrap());
    let navigation_authorizations = Arc::new(FileNavigationAuthorizations::new());
    let grants = FileAccessGrantSource::new(
        capability.issue_grant(),
        Arc::clone(&navigation_authorizations),
    );
    let provider = BrowserFileProvider::new(
        Arc::clone(&capability),
        grants,
        Arc::clone(&navigation_authorizations),
        u64::MAX,
    );
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let url = Url::from_file_path(&file).unwrap();

    for read_first_chunk in [false, true] {
        let mut request = Request::get(url.clone());
        request.set_mode(net::RequestMode::Navigate).unwrap();
        navigation_authorizations.authorize(request.id()).unwrap();
        let resource = runtime.block_on(provider.open(&request, &url)).unwrap();
        let SchemeBody::Stream(mut body) = resource.body else {
            panic!("GET requests should receive a live file body");
        };

        if read_first_chunk {
            let chunk = runtime.block_on(body.next()).unwrap().unwrap();
            assert!(!chunk.is_empty());
        }
        drop(body);

        runtime.block_on(wait_for_readers_to_finish(&provider.read_admission));
        assert_eq!(
            provider.read_admission.active.available_permits(),
            MAX_CONCURRENT_FILE_READS
        );
    }

    drop(runtime);
    drop(provider);
    drop(capability);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn file_body_fails_when_opened_length_does_not_match_live_bytes() {
    let directory = std::env::temp_dir().join(format!(
        "byui-browser-content-length-{}-{}",
        std::process::id(),
        NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&directory).unwrap();
    let file = directory.join("page.html");
    let canonical_directory = fs::canonicalize(&directory).unwrap();
    let selection = TrustedDirectorySelection::from_verified_open_directory(
        canonical_directory,
        Dir::open_ambient_dir(&directory, ambient_authority()).unwrap(),
    )
    .unwrap();
    let capability =
        Arc::new(FileAccessCapability::from_trusted_directory_selections([selection]).unwrap());
    let navigation_authorizations = Arc::new(FileNavigationAuthorizations::new());
    let grants = FileAccessGrantSource::new(
        capability.issue_grant(),
        Arc::clone(&navigation_authorizations),
    );
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let url = Url::from_file_path(&file).unwrap();

    for actual in [b"a".as_slice(), b"abc".as_slice()] {
        fs::write(&file, b"ab").unwrap();
        let replacement = actual.to_vec();
        let replacement_path = file.clone();
        let mut provider = BrowserFileProvider::new(
            Arc::clone(&capability),
            grants.clone(),
            Arc::clone(&navigation_authorizations),
            u64::MAX,
        );
        provider.post_setup_hook = Some(Arc::new(move || {
            fs::write(&replacement_path, &replacement).unwrap();
        }));
        let mut request = Request::get(url.clone());
        request.set_mode(net::RequestMode::Navigate).unwrap();
        navigation_authorizations.authorize(request.id()).unwrap();
        let resource = runtime.block_on(provider.open(&request, &url)).unwrap();
        assert_eq!(
            resource
                .headers
                .iter()
                .find(|(name, _)| name == "content-length")
                .unwrap()
                .1,
            HeaderValue::from_static("2")
        );
        let SchemeBody::Stream(mut body) = resource.body else {
            panic!("GET requests should receive a live file body");
        };

        let mut received = Vec::new();
        let error = loop {
            match runtime.block_on(body.next()).unwrap() {
                Ok(chunk) => received.extend_from_slice(&chunk),
                Err(error) => break error,
            }
        };
        assert!(matches!(
            error,
            RequestError::Provider {
                provider: "file",
                kind: ProviderErrorKind::Io
            }
        ));
        assert_eq!(received, actual[..actual.len().min(2)]);
    }

    drop(runtime);
    drop(capability);
    fs::remove_dir_all(directory).unwrap();
}

async fn wait_for_readers_to_finish(admission: &FileReadAdmission) {
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while admission.active.available_permits() != MAX_CONCURRENT_FILE_READS {
        assert!(
            std::time::Instant::now() < deadline,
            "dropped body should release its file-reader permit"
        );
        tokio::task::yield_now().await;
    }
}
