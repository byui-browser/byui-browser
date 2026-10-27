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
    FileAccessAuditEvent, FileAccessGrantSource, FileNavigationAuthorizations,
    admission::{FileReadAdmission, MAX_CONCURRENT_FILE_READS, MAX_PENDING_FILE_READS},
    provider::BrowserFileProvider,
    reader::{BufferedChunk, FILE_READ_CHUNK_SIZE, FileBufferBudget},
    session::FileAccessCancellation,
    stream::FileBodyStream,
};

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

#[test]
fn file_read_admission_enforces_reader_and_queue_limits_and_cancels_queued_work() {
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

        let admission = FileReadAdmission::new(MAX_CONCURRENT_FILE_READS, MAX_PENDING_FILE_READS);
        let session_cancellation = FileAccessCancellation::new();
        let signals = (0..MAX_CONCURRENT_FILE_READS)
            .map(|_| AbortSignal::new())
            .collect::<Vec<_>>();
        let active = futures_util::future::join_all(
            signals
                .iter()
                .map(|signal| admission.acquire(signal, &session_cancellation)),
        )
        .await;
        let active = active
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .expect("active reader permits should be available");
        let queued = (0..MAX_PENDING_FILE_READS)
            .map(|_| {
                let admission = admission.clone();
                let cancellation = session_cancellation.clone();
                tokio::spawn(
                    async move { admission.acquire(&AbortSignal::new(), &cancellation).await },
                )
            })
            .collect::<Vec<_>>();
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
        session_cancellation.cancel();
        for queued in queued {
            assert!(matches!(queued.await.unwrap(), Err(RequestError::Aborted)));
        }
        drop(active);
        assert_eq!(
            admission.active.available_permits(),
            MAX_CONCURRENT_FILE_READS
        );
        assert_eq!(
            admission.pending.available_permits(),
            MAX_PENDING_FILE_READS
        );
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
    let directory = fs::canonicalize(directory).unwrap();
    let file = directory.join("page.html");
    fs::write(&file, b"prepared content").unwrap();
    let selection = TrustedDirectorySelection::from_verified_open_directory(
        directory.clone(),
        Dir::open_ambient_dir(&directory, ambient_authority()).unwrap(),
    )
    .unwrap();
    let capability =
        Arc::new(FileAccessCapability::from_trusted_directory_selections([selection]).unwrap());
    let navigation_authorizations = Arc::new(FileNavigationAuthorizations::new());
    let grants = FileAccessGrantSource::new(
        capability.issue_grant(),
        Arc::clone(&navigation_authorizations),
        None,
    );
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let url = Url::from_file_path(&file).unwrap();

    for method in [net::Method::GET, net::Method::HEAD] {
        let hook_grants = grants.clone();
        let mut provider = BrowserFileProvider::new(
            Arc::clone(&capability),
            grants.clone(),
            Arc::clone(&navigation_authorizations),
            crate::FileAccessLimits {
                max_file_size: u64::MAX,
                max_session_bytes: u64::MAX,
                ..crate::FileAccessLimits::default()
            },
            crate::FileAccessContentPolicy::StaticDocument,
        );
        provider.after_open_hook = Some(Arc::new(move || hook_grants.revoke()));
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
            limits: provider.limits,
            session_budget: provider.session_budget.clone(),
            buffer_budget: provider.buffer_budget.clone(),
            content_policy: provider.content_policy,
            after_open_hook: None,
            before_read_hook: None,
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
    let buffer_budget = FileBufferBudget::new(1);
    assert!(buffer_budget.try_reserve(1));
    sender
        .try_send(Ok(BufferedChunk::new(vec![b'x'], 1, buffer_budget)))
        .unwrap();
    grant.revoke();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let runtime_guard = runtime.enter();
    let mut body = FileBodyStream {
        receiver,
        cancelled: Arc::new(AtomicBool::new(false)),
        session_cancellation: FileAccessCancellation::new(),
        lease,
        reported_session_cancellation: false,
        deadline: std::time::Instant::now() + Duration::from_secs(1),
        deadline_timer: Box::pin(tokio::time::sleep(Duration::from_secs(1))),
        reported_timeout: false,
    };
    drop(runtime_guard);

    assert!(matches!(
        runtime.block_on(body.next()),
        Some(Err(RequestError::Aborted))
    ));
}

#[test]
fn timed_out_file_body_reports_resource_exhaustion() {
    let capability = FileAccessCapability::from_trusted_directory_selections([]).unwrap();
    let grant = capability.issue_grant();
    let lease = grant.lease().unwrap();
    let (_sender, receiver) = mpsc::channel(1);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let runtime_guard = runtime.enter();
    let mut body = FileBodyStream {
        receiver,
        cancelled: Arc::new(AtomicBool::new(false)),
        session_cancellation: FileAccessCancellation::new(),
        lease,
        reported_session_cancellation: false,
        deadline: std::time::Instant::now(),
        deadline_timer: Box::pin(tokio::time::sleep(Duration::ZERO)),
        reported_timeout: false,
    };
    drop(runtime_guard);

    assert!(matches!(
        runtime.block_on(body.next()),
        Some(Err(RequestError::Provider {
            provider: "file",
            kind: ProviderErrorKind::ResourceExhausted,
        }))
    ));
}

#[test]
fn buffered_chunks_return_capacity_when_consumed_or_dropped() {
    let budget = FileBufferBudget::new(3);
    assert!(budget.try_reserve(3));
    let chunk = BufferedChunk::new(vec![b'x'; 3], 3, budget.clone());
    assert_eq!(budget.available(), 0);
    assert_eq!(chunk.into_bytes(), b"xxx");
    assert_eq!(budget.available(), 3);

    assert!(budget.try_reserve(3));
    drop(BufferedChunk::new(vec![b'x'; 3], 3, budget.clone()));
    assert_eq!(budget.available(), 3);
}

#[test]
fn expiry_cancels_queued_delivery_and_audits_the_lifecycle() {
    let capability = FileAccessCapability::from_trusted_directory_selections([]).unwrap();
    let navigation_authorizations = Arc::new(FileNavigationAuthorizations::new());
    let source = FileAccessGrantSource::new(
        capability.issue_grant(),
        navigation_authorizations,
        Some(Duration::from_millis(20)),
    );
    let (_, lease, cancellation) = source.snapshot().unwrap();
    let (sender, receiver) = mpsc::channel(1);
    let buffer_budget = FileBufferBudget::new(1);
    assert!(buffer_budget.try_reserve(1));
    sender
        .try_send(Ok(BufferedChunk::new(vec![b'x'], 1, buffer_budget)))
        .unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let runtime_guard = runtime.enter();
    let mut body = FileBodyStream {
        receiver,
        cancelled: Arc::new(AtomicBool::new(false)),
        session_cancellation: cancellation,
        lease,
        reported_session_cancellation: false,
        deadline: std::time::Instant::now() + Duration::from_secs(1),
        deadline_timer: Box::pin(tokio::time::sleep(Duration::from_secs(1))),
        reported_timeout: false,
    };
    drop(runtime_guard);

    let wait_deadline = std::time::Instant::now() + Duration::from_secs(1);
    while source.is_active() {
        assert!(
            std::time::Instant::now() < wait_deadline,
            "grant did not expire"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(matches!(
        runtime.block_on(body.next()),
        Some(Err(RequestError::Aborted))
    ));
    assert_eq!(source.audit_events(), vec![FileAccessAuditEvent::Expired]);
}

#[test]
fn replacement_and_shutdown_are_audited_and_fail_closed() {
    let capability = FileAccessCapability::from_trusted_directory_selections([]).unwrap();
    let navigation_authorizations = Arc::new(FileNavigationAuthorizations::new());
    let source =
        FileAccessGrantSource::new(capability.issue_grant(), navigation_authorizations, None);

    source.replace(capability.issue_grant());
    assert!(source.is_active());
    source.shutdown();
    assert!(!source.is_active());
    assert_eq!(
        source.audit_events(),
        vec![
            FileAccessAuditEvent::Replaced,
            FileAccessAuditEvent::Shutdown
        ]
    );
}

#[test]
fn slow_read_deadline_aborts_the_body_and_releases_its_reader() {
    let directory = test_directory("slow-read-deadline");
    let file = directory.join("page.html");
    fs::write(&file, b"slow").unwrap();
    let (capability, grants, navigation_authorizations) = provider_authority(&directory);
    let mut provider = BrowserFileProvider::new(
        capability,
        grants,
        Arc::clone(&navigation_authorizations),
        crate::FileAccessLimits {
            max_file_size: u64::MAX,
            max_session_bytes: u64::MAX,
            max_read_duration: Duration::from_millis(200),
            max_buffered_bytes: FILE_READ_CHUNK_SIZE as u64,
            grant_lifetime: None,
        },
        crate::FileAccessContentPolicy::StaticDocument,
    );
    provider.before_read_hook = Some(Arc::new(|| std::thread::sleep(Duration::from_millis(300))));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let url = Url::from_file_path(&file).unwrap();
    let mut request = Request::get(url.clone());
    request.set_mode(net::RequestMode::Navigate).unwrap();
    navigation_authorizations.authorize(request.id()).unwrap();

    let resource = runtime.block_on(provider.open(&request, &url)).unwrap();
    let SchemeBody::Stream(mut body) = resource.body else {
        panic!("GET requests should receive a live file body");
    };
    assert!(matches!(
        runtime.block_on(body.next()),
        Some(Err(RequestError::Provider {
            provider: "file",
            kind: ProviderErrorKind::ResourceExhausted,
        }))
    ));
    runtime.block_on(wait_for_readers_to_finish(&provider.read_admission));

    drop(runtime);
    drop(provider);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn full_channel_deadline_releases_the_blocking_reader() {
    let directory = test_directory("full-channel-deadline");
    let file = directory.join("page.html");
    fs::write(&file, vec![b'x'; FILE_READ_CHUNK_SIZE * 3]).unwrap();
    let (capability, grants, navigation_authorizations) = provider_authority(&directory);
    let provider = BrowserFileProvider::new(
        capability,
        grants,
        Arc::clone(&navigation_authorizations),
        crate::FileAccessLimits {
            max_file_size: u64::MAX,
            max_session_bytes: u64::MAX,
            max_read_duration: Duration::from_millis(200),
            max_buffered_bytes: (FILE_READ_CHUNK_SIZE * 3) as u64,
            grant_lifetime: None,
        },
        crate::FileAccessContentPolicy::StaticDocument,
    );
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let url = Url::from_file_path(&file).unwrap();
    let mut request = Request::get(url.clone());
    request.set_mode(net::RequestMode::Navigate).unwrap();
    navigation_authorizations.authorize(request.id()).unwrap();

    let resource = runtime.block_on(provider.open(&request, &url)).unwrap();
    let SchemeBody::Stream(mut body) = resource.body else {
        panic!("GET requests should receive a live file body");
    };
    runtime.block_on(wait_for_readers_to_finish(&provider.read_admission));
    assert!(matches!(
        runtime.block_on(body.next()),
        Some(Err(RequestError::Provider {
            provider: "file",
            kind: ProviderErrorKind::ResourceExhausted,
        }))
    ));

    drop(runtime);
    drop(provider);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn dropping_unread_or_partially_read_body_releases_reader_admission() {
    let directory = std::env::temp_dir().join(format!(
        "byui-browser-body-drop-{}-{}",
        std::process::id(),
        NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&directory).unwrap();
    let directory = fs::canonicalize(directory).unwrap();
    let file = directory.join("page.html");
    fs::write(&file, vec![b'x'; FILE_READ_CHUNK_SIZE * 4]).unwrap();
    let selection = TrustedDirectorySelection::from_verified_open_directory(
        directory.clone(),
        Dir::open_ambient_dir(&directory, ambient_authority()).unwrap(),
    )
    .unwrap();
    let capability =
        Arc::new(FileAccessCapability::from_trusted_directory_selections([selection]).unwrap());
    let navigation_authorizations = Arc::new(FileNavigationAuthorizations::new());
    let grants = FileAccessGrantSource::new(
        capability.issue_grant(),
        Arc::clone(&navigation_authorizations),
        None,
    );
    let provider = BrowserFileProvider::new(
        Arc::clone(&capability),
        grants,
        Arc::clone(&navigation_authorizations),
        crate::FileAccessLimits {
            max_file_size: u64::MAX,
            max_session_bytes: u64::MAX,
            ..crate::FileAccessLimits::default()
        },
        crate::FileAccessContentPolicy::StaticDocument,
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
fn file_body_uses_captured_framing_for_live_file_mutations() {
    let directory = std::env::temp_dir().join(format!(
        "byui-browser-content-length-{}-{}",
        std::process::id(),
        NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&directory).unwrap();
    let directory = fs::canonicalize(directory).unwrap();
    let file = directory.join("page.html");
    let selection = TrustedDirectorySelection::from_verified_open_directory(
        directory.clone(),
        Dir::open_ambient_dir(&directory, ambient_authority()).unwrap(),
    )
    .unwrap();
    let capability =
        Arc::new(FileAccessCapability::from_trusted_directory_selections([selection]).unwrap());
    let navigation_authorizations = Arc::new(FileNavigationAuthorizations::new());
    let grants = FileAccessGrantSource::new(
        capability.issue_grant(),
        Arc::clone(&navigation_authorizations),
        None,
    );
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let url = Url::from_file_path(&file).unwrap();

    for (actual, expect_error) in [
        (b"cd".as_slice(), false),
        (b"a".as_slice(), true),
        (b"abc".as_slice(), true),
    ] {
        fs::write(&file, b"ab").unwrap();
        let replacement = actual.to_vec();
        let replacement_path = file.clone();
        let mut provider = BrowserFileProvider::new(
            Arc::clone(&capability),
            grants.clone(),
            Arc::clone(&navigation_authorizations),
            crate::FileAccessLimits {
                max_file_size: u64::MAX,
                max_session_bytes: u64::MAX,
                ..crate::FileAccessLimits::default()
            },
            crate::FileAccessContentPolicy::StaticDocument,
        );
        provider.after_open_hook = Some(Arc::new(move || {
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
        let result = loop {
            match runtime.block_on(body.next()) {
                Some(Ok(chunk)) => received.extend_from_slice(&chunk),
                Some(Err(error)) => break Err(error),
                // A clean end is permitted only when the in-place write kept
                // the captured length unchanged.
                None => break Ok(()),
            }
        };
        if expect_error {
            assert!(matches!(
                result,
                Err(RequestError::Provider {
                    provider: "file",
                    kind: ProviderErrorKind::Io
                })
            ));
        } else {
            assert!(result.is_ok());
        }
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

fn test_directory(label: &str) -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "byui-browser-{label}-{}-{}",
        std::process::id(),
        NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&directory).unwrap();
    fs::canonicalize(directory).unwrap()
}

fn provider_authority(
    directory: &std::path::Path,
) -> (
    Arc<FileAccessCapability>,
    FileAccessGrantSource,
    Arc<FileNavigationAuthorizations>,
) {
    let canonical_directory = fs::canonicalize(directory).unwrap();
    let selection = TrustedDirectorySelection::from_verified_open_directory(
        canonical_directory,
        Dir::open_ambient_dir(directory, ambient_authority()).unwrap(),
    )
    .unwrap();
    let capability =
        Arc::new(FileAccessCapability::from_trusted_directory_selections([selection]).unwrap());
    let navigation_authorizations = Arc::new(FileNavigationAuthorizations::new());
    let grants = FileAccessGrantSource::new(
        capability.issue_grant(),
        Arc::clone(&navigation_authorizations),
        None,
    );
    (capability, grants, navigation_authorizations)
}
