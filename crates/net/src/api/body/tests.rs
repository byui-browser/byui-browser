use std::io;

use bytes::Bytes;
use futures_util::stream;
use http::header::HeaderValue;

use crate::api::cancellation::AbortController;

use super::*;

#[test]
/// Verifies body constructors expose their replayability and length metadata.
fn body_constructors_report_replayability_and_length() {
    let null = Body::null();
    assert!(null.is_null());
    assert!(!null.is_replayable());
    assert_eq!(null.length(), Some(0));

    let bytes = Body::from_bytes(b"hello".to_vec());
    assert!(!bytes.is_null());
    assert!(bytes.is_replayable());
    assert_eq!(bytes.length(), Some(5));

    let one_shot = Body::one_shot_bytes(b"hello".to_vec());
    assert!(!one_shot.is_null());
    assert!(!one_shot.is_replayable());
    assert_eq!(one_shot.length(), Some(5));

    let stream = Body::stream(stream::iter(vec![Ok::<Vec<u8>, io::Error>(
        b"hello".to_vec(),
    )]));
    assert!(!stream.is_null());
    assert!(!stream.is_replayable());
    assert_eq!(stream.length(), None);
}

#[test]
/// Verifies URL-encoded bodies expose metadata and support ArrayBuffer reads.
fn url_encoded_body_supports_array_buffer_consumption() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let body = Body::from_url_encoded(b"name=Ada+Lovelace".to_vec());

    assert_eq!(
        body.content_type(),
        Some(HeaderValue::from_static(
            "application/x-www-form-urlencoded;charset=UTF-8"
        ))
    );
    assert_eq!(body.length(), Some(17));
    assert_eq!(
        runtime.block_on(body.array_buffer()).unwrap(),
        b"name=Ada+Lovelace"
    );
}

#[test]
/// Verifies body equality distinguishes value, null, and one-shot identity.
fn body_equality_matches_replayability_rules() {
    assert_eq!(Body::null(), Body::null());
    assert_eq!(
        Body::from_bytes(b"same".to_vec()),
        Body::from_bytes(b"same")
    );
    assert_ne!(
        Body::from_bytes(b"same".to_vec()),
        Body::from_bytes(b"other")
    );

    let one_shot = Body::one_shot_bytes(b"same".to_vec());
    assert_eq!(one_shot, one_shot.clone());
    assert_ne!(one_shot, Body::one_shot_bytes(b"same".to_vec()));
}

#[test]
/// Verifies debug output reports body state without consuming the body.
fn body_debug_reports_state_without_consuming() {
    let body = Body::from_bytes(b"debug".to_vec());
    let debug = format!("{body:?}");

    assert!(debug.contains("is_null: false"));
    assert!(debug.contains("replayable: true"));
    assert!(debug.contains("length: Some(5)"));
    assert!(!body.is_used());
    assert!(!body.is_locked());
}

#[test]
/// Verifies replayable body clones consume independently.
fn replayable_clones_have_independent_consumption_state() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let body = Body::from_bytes(b"replayable".to_vec());
    let clone = body.clone();

    assert_eq!(runtime.block_on(body.bytes()).unwrap(), b"replayable");
    assert_eq!(runtime.block_on(clone.bytes()).unwrap(), b"replayable");
}

#[test]
/// Verifies one-shot body clones share consumption state.
fn one_shot_clones_share_consumption_state() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let body = Body::one_shot_bytes(b"once".to_vec());
    let clone = body.clone();

    assert!(!body.is_locked());
    assert_eq!(runtime.block_on(clone.bytes()).unwrap(), b"once");
    assert!(body.is_locked());
    assert!(body.is_used());
    assert!(matches!(
        runtime.block_on(body.bytes()),
        Err(RequestError::BodyAlreadyUsed)
    ));
}

#[test]
/// Verifies a request body distinguishes transport consumption from body reads.
fn one_shot_transport_consumption_reports_body_already_consumed() {
    let body = Body::one_shot_bytes(b"once".to_vec());
    let clone = body.clone();

    body.into_reqwest_body().unwrap();

    assert!(clone.is_used());
    assert!(matches!(
        clone.into_reqwest_body(),
        Err(RequestError::BodyAlreadyConsumed)
    ));
}

#[test]
/// Verifies body constructors preserve text and blob metadata.
fn body_helpers_preserve_text_and_blob_metadata() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let text = Body::from_text("hello");
    assert_eq!(runtime.block_on(text.text()).unwrap(), "hello");

    let blob = Blob::new(
        b"blob".to_vec(),
        Some(HeaderValue::from_static("application/octet-stream")),
    );
    let blob = runtime.block_on(Body::from_blob(blob).blob()).unwrap();
    assert_eq!(blob.bytes(), b"blob");
    assert_eq!(
        blob.media_type(),
        Some(&HeaderValue::from_static("application/octet-stream"))
    );
}

#[test]
/// Verifies body helpers decode bytes and preserve form entries.
fn body_consumption_helpers_decode_parse_and_preserve_form_entries() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let latin = Body::from_bytes_with_content_type(
        vec![0x68, 0xe9],
        Some(HeaderValue::from_static("text/plain; charset=iso-8859-1")),
    );
    assert_eq!(runtime.block_on(latin.text()).unwrap(), "h\u{e9}");

    let json = Body::from_text("{\"answer\":42}");
    let value: serde_json::Value = runtime.block_on(json.json()).unwrap();
    assert_eq!(value["answer"], 42);

    let form = Body::from_bytes_with_content_type(
        b"name=Ada&name=Grace".to_vec(),
        Some(HeaderValue::from_static(
            "application/x-www-form-urlencoded;charset=UTF-8",
        )),
    );
    let form = runtime.block_on(form.form_data()).unwrap();
    assert_eq!(form.entries().count(), 2);
    assert!(matches!(
        form.entries().next(),
        Some(("name", FormDataEntry::Text(value))) if value == "Ada"
    ));
}

#[test]
/// Verifies multipart parsing creates text and file entries.
fn multipart_form_data_parses_text_and_file_parts() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let body = Body::from_bytes_with_content_type(
        b"--boundary\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nnotes\r\n--boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.txt\"\r\nContent-Type: text/plain\r\n\r\nhello\r\n--boundary--\r\n".to_vec(),
        Some(HeaderValue::from_static("multipart/form-data; boundary=boundary")),
    );
    let form = runtime.block_on(body.form_data()).unwrap();
    let entries: Vec<_> = form.entries().collect();
    assert!(matches!(entries[0], ("title", FormDataEntry::Text(value)) if value == "notes"));
    assert!(
        matches!(entries[1], ("file", FormDataEntry::File { blob, filename: Some(filename) }) if blob.bytes() == b"hello" && filename == "a.txt")
    );
}

#[test]
/// Verifies form-data serialization selects the correct encoding.
fn form_data_constructor_selects_url_encoded_or_multipart() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut text = FormData::new();
    text.append("name", "Ada Lovelace");
    let body = Body::from_form_data(text);
    assert_eq!(
        body.content_type(),
        Some(HeaderValue::from_static(
            "application/x-www-form-urlencoded;charset=UTF-8"
        ))
    );
    assert_eq!(
        runtime.block_on(body.bytes()).unwrap(),
        b"name=Ada+Lovelace"
    );

    let mut file = FormData::new();
    file.append_file(
        "file",
        Blob::new(b"hello".to_vec(), None),
        Some("a.txt".into()),
    );
    let body = Body::from_form_data(file);
    assert!(
        body.content_type()
            .unwrap()
            .as_bytes()
            .starts_with(b"multipart/form-data; boundary=")
    );
}

#[test]
/// Verifies a stream failure marks shared body state as used.
fn response_stream_failure_marks_the_shared_body_used() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let body = Body::stream(stream::once(async {
        Err(io::Error::other("stream failure"))
    }));
    let clone = body.clone();

    assert!(matches!(
        runtime.block_on(clone.bytes()),
        Err(RequestError::Transport(_))
    ));
    assert!(matches!(
        runtime.block_on(body.bytes()),
        Err(RequestError::BodyAlreadyUsed)
    ));
}

#[test]
/// Verifies a response stream records a failed terminal state.
fn response_stream_failure_records_failed_state() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let body = Body::stream(stream::once(async {
        Err(io::Error::other("stream failure"))
    }));

    assert!(matches!(
        runtime.block_on(body.clone().bytes()),
        Err(RequestError::Transport(_))
    ));
    let debug = format!("{body:?}");
    assert!(debug.contains("used: true"));
    assert!(debug.contains("completed: true"));
    assert!(debug.contains("failed: true"));
    assert!(debug.contains("aborted: false"));
}

#[test]
/// Verifies cancellation marks a live response body as aborted.
fn response_stream_abort_records_aborted_state() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let controller = AbortController::new();
    let body = Body::from_stream_with_signal(
        stream::pending::<Result<Bytes, RequestError>>(),
        controller.signal(),
    );
    controller.abort();

    assert!(matches!(
        runtime.block_on(body.clone().bytes()),
        Err(RequestError::Aborted)
    ));
    let debug = format!("{body:?}");
    assert!(debug.contains("used: true"));
    assert!(debug.contains("completed: true"));
    assert!(debug.contains("failed: false"));
    assert!(debug.contains("aborted: true"));
}

#[test]
/// Verifies malformed form-data inputs return stable body errors.
fn malformed_form_data_returns_typed_errors() {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let missing_boundary = Body::from_bytes_with_content_type(
        b"field=value".to_vec(),
        Some(HeaderValue::from_static("multipart/form-data")),
    );
    assert!(matches!(
        runtime.block_on(missing_boundary.form_data()),
        Err(RequestError::BodyFormData(message)) if message.contains("Content-Type")
    ));

    let missing_disposition = Body::from_bytes_with_content_type(
        b"--boundary\r\nContent-Type: text/plain\r\n\r\nvalue\r\n--boundary--\r\n".to_vec(),
        Some(HeaderValue::from_static(
            "multipart/form-data; boundary=boundary",
        )),
    );
    assert!(matches!(
        runtime.block_on(missing_disposition.form_data()),
        Err(RequestError::BodyFormData(message)) if message.contains("disposition")
    ));
}
