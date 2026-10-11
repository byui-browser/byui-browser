use std::io;

use futures_util::stream;
use http::header::HeaderValue;

use super::*;

#[test]
fn replayable_clones_have_independent_consumption_state() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let body = Body::from_bytes(b"replayable".to_vec());
    let clone = body.clone();

    assert_eq!(runtime.block_on(body.bytes()).unwrap(), b"replayable");
    assert_eq!(runtime.block_on(clone.bytes()).unwrap(), b"replayable");
}

#[test]
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
