use std::io;

use futures_util::stream;
use http::header::HeaderValue;

use super::*;

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
    assert!(!body.is_locked());
    assert!(body.is_used());
    assert!(matches!(
        runtime.block_on(body.bytes()),
        Err(RequestError::BodyAlreadyUsed)
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
/// Verifies JSON always uses UTF-8 and accepts its byte-order mark.
fn json_decoding_uses_utf8_regardless_of_content_type_charset() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let body = Body::from_bytes_with_content_type(
        "{\"name\":\"café\"}".as_bytes().to_vec(),
        Some(HeaderValue::from_static(
            "application/json; charset=iso-8859-1",
        )),
    );
    let value: serde_json::Value = runtime.block_on(body.json()).unwrap();
    assert_eq!(value["name"], "café");

    let body = Body::from_array_buffer([b"\xef\xbb\xbf".as_slice(), b"{\"ok\":true}"].concat());
    let value: serde_json::Value = runtime.block_on(body.json()).unwrap();
    assert_eq!(value["ok"], true);
}

#[test]
/// Verifies typed-array inputs are copied into replayable body storage.
fn typed_array_input_is_replayable_and_length_aware() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let source = [0, 1, 0xff];
    let body = Body::from_typed_array(&source);
    let clone = body.try_clone().unwrap();

    assert_eq!(body.length(), Some(3));
    assert_eq!(runtime.block_on(body.bytes()).unwrap(), source);
    assert_eq!(runtime.block_on(clone.bytes()).unwrap(), source);
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
/// Verifies multipart parsing preserves arbitrary file bytes and extended names.
fn multipart_form_data_is_binary_safe_and_decodes_rfc5987_filenames() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut bytes = b"--boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"fallback.bin\"; filename*=UTF-8''caf%C3%A9.bin\r\nContent-Type: application/octet-stream\r\n\r\n".to_vec();
    bytes.extend_from_slice(&[0, 0xff, b'\r', b'\n', 1]);
    bytes.extend_from_slice(b"\r\n--boundary--\r\n");
    let body = Body::from_bytes_with_content_type(
        bytes,
        Some(HeaderValue::from_static(
            "multipart/form-data; boundary=boundary",
        )),
    );

    let form = runtime.block_on(body.form_data()).unwrap();
    let entry = form.entries().next().unwrap();
    assert!(
        matches!(entry, ("file", FormDataEntry::File { blob, filename: Some(filename) })
            if filename == "café.bin" && blob.bytes() == [0, 0xff, b'\r', b'\n', 1])
    );
}

#[test]
/// Verifies multipart serialization emits an RFC 5987 extended filename.
fn multipart_serialization_emits_extended_non_ascii_filename() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut form = FormData::new();
    form.append("café", "value");
    form.append_file(
        "upload",
        Blob::new([0, 0xff, 1], None),
        Some("café.bin".into()),
    );
    let body = Body::from_form_data(form);
    let bytes = runtime.block_on(body.clone().bytes()).unwrap();

    assert!(
        bytes
            .windows(b"filename*=UTF-8''caf%C3%A9.bin".len())
            .any(|window| window == b"filename*=UTF-8''caf%C3%A9.bin")
    );
    assert!(bytes.windows(3).any(|window| window == [0, 0xff, 1]));
    let parsed = runtime.block_on(body.form_data()).unwrap();
    assert!(matches!(
        parsed.entries().next(),
        Some(("café", FormDataEntry::Text(value))) if value == "value"
    ));
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
/// Verifies a live body tee delivers identical ordered bytes to both branches.
fn live_body_tee_delivers_bounded_independent_branches() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut body = Body::from_stream(stream::iter([
        Ok(Bytes::from_static(b"first")),
        Ok(Bytes::from_static(b"second")),
    ]));
    body.set_content_type(Some(HeaderValue::from_static("text/plain")));
    body.set_length(Some(11));
    let clone = body.tee().unwrap();

    let (left, right) = runtime.block_on(async { tokio::join!(body.bytes(), clone.bytes()) });
    assert_eq!(left.unwrap(), b"firstsecond");
    assert_eq!(right.unwrap(), b"firstsecond");
}

#[test]
/// Verifies dropping one tee branch lets the remaining branch finish.
fn dropping_one_tee_branch_releases_backpressure() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut body = Body::from_stream(stream::iter([
        Ok(Bytes::from_static(b"one")),
        Ok(Bytes::from_static(b"two")),
    ]));
    let clone = body.tee().unwrap();
    drop(clone);

    assert_eq!(runtime.block_on(body.bytes()).unwrap(), b"onetwo");
}

#[test]
/// Verifies source failures are delivered to both active tee branches.
fn tee_forwards_source_failure_to_both_branches() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut body = Body::from_stream(stream::iter([Err(RequestError::Transport(
        "failed".into(),
    ))]));
    let clone = body.tee().unwrap();

    let (left, right) = runtime.block_on(async { tokio::join!(body.bytes(), clone.bytes()) });
    assert!(matches!(left, Err(RequestError::Transport(message)) if message == "failed"));
    assert!(matches!(right, Err(RequestError::Transport(message)) if message == "failed"));
}

#[test]
/// Verifies fallible cloning rejects a disturbed stream without changing it.
fn fallible_clone_rejects_a_disturbed_stream() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut body = Body::once(b"body".to_vec());
    assert_eq!(
        runtime.block_on(body.next()).unwrap().unwrap(),
        b"body".as_slice()
    );
    assert!(matches!(
        body.try_clone(),
        Err(RequestError::BodyNotCloneable)
    ));
    assert!(runtime.block_on(body.next()).is_none());
}

#[test]
/// Verifies direct polling disturbs every view sharing a one-shot source.
fn direct_stream_read_blocks_later_helper_consumption() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut body = Body::one_shot_bytes(b"body".to_vec());
    let clone = body.clone();

    assert_eq!(
        runtime.block_on(body.next()).unwrap().unwrap(),
        b"body".as_slice()
    );
    assert_eq!(body.status(), BodyStatus::Complete);
    assert!(matches!(
        runtime.block_on(clone.bytes()),
        Err(RequestError::BodyAlreadyUsed)
    ));
}

#[test]
/// Verifies request transport handoff is an authoritative terminal transition.
fn transport_handoff_marks_shared_one_shot_state_transferred() {
    let body = Body::one_shot_bytes(b"body".to_vec());
    let clone = body.clone();

    body.into_reqwest_body().unwrap();
    assert_eq!(clone.status(), BodyStatus::Transferred);
    assert!(matches!(
        clone.try_clone(),
        Err(RequestError::BodyNotCloneable)
    ));
}

#[test]
/// Verifies an abort reaches both active branches and records terminal state.
fn abort_terminates_both_tee_branches() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let abort = crate::AbortController::new();
    let mut body = Body::from_stream_with_signal(
        stream::pending::<Result<Bytes, RequestError>>(),
        abort.signal(),
    );
    let mut clone = body.tee().unwrap();
    abort.abort();

    let (left, right) = runtime.block_on(async { tokio::join!(body.next(), clone.next()) });
    assert!(matches!(left, Some(Err(RequestError::Aborted))));
    assert!(matches!(right, Some(Err(RequestError::Aborted))));
    assert_eq!(body.status(), BodyStatus::Aborted);
    assert_eq!(clone.status(), BodyStatus::Aborted);
}
