use super::InternalResponse;
use crate::api::{
    providers::{SchemeBody, SchemeResource},
    request::{Origin, Request},
};
use crate::policy::FetchScheme;

/// Builds minimal provider metadata for response-view tests.
fn resource(url: &str) -> SchemeResource {
    SchemeResource {
        url: reqwest::Url::parse(url).unwrap(),
        status: 200,
        status_text: Some("OK".into()),
        headers: Vec::new(),
        media_type: None,
        origin: Origin::opaque(),
        body: SchemeBody::Bytes(Vec::new()),
    }
}

#[test]
/// Verifies a provider cannot return metadata for another scheme.
fn scheme_metadata_must_match_the_selected_dispatcher() {
    let request = Request::get("data:,ok");
    let result = InternalResponse::from_scheme_resource(
        resource("blob:https://example.test/id"),
        &request,
        FetchScheme::Data,
    );

    assert!(matches!(
        result,
        Err(crate::RequestError::Provider {
            provider: "data",
            kind: crate::ProviderErrorKind::MalformedMetadata
        })
    ));
}

#[test]
/// Verifies conflicting media-type metadata is rejected.
fn conflicting_media_type_metadata_fails_closed() {
    let mut resource = resource("data:,ok");
    resource.media_type = Some("text/plain".parse().unwrap());
    resource.headers.push((
        http::header::CONTENT_TYPE,
        "application/json".parse().unwrap(),
    ));
    let result = InternalResponse::from_scheme_resource(
        resource,
        &Request::get("data:,ok"),
        FetchScheme::Data,
    );

    assert!(matches!(
        result,
        Err(crate::RequestError::Provider {
            provider: "data",
            kind: crate::ProviderErrorKind::MalformedMetadata
        })
    ));
}

#[test]
fn scheme_resources_accept_each_supported_scheme() {
    let request = Request::get("https://client.test/");
    for (url, scheme) in [
        ("http://example.test/resource", FetchScheme::Http),
        ("https://example.test/resource", FetchScheme::Https),
        ("data:,resource", FetchScheme::Data),
        ("blob:https://example.test/id", FetchScheme::Blob),
        ("file:///tmp/resource", FetchScheme::File),
        ("byui://welcome/page", FetchScheme::Byui),
        ("about:blank", FetchScheme::About),
    ] {
        assert!(
            InternalResponse::from_scheme_resource(resource(url), &request, scheme).is_ok(),
            "{url} should match {scheme:?}"
        );
    }
}

#[test]
fn scheme_resource_rejects_invalid_status_text_media_type_and_status() {
    let request = Request::get("data:,ok");

    let mut invalid_status_text = resource("data:,ok");
    invalid_status_text.status_text = Some("not\nok".into());
    assert!(matches!(
        InternalResponse::from_scheme_resource(invalid_status_text, &request, FetchScheme::Data),
        Err(crate::RequestError::Provider {
            provider: "data",
            kind: crate::ProviderErrorKind::MalformedMetadata
        })
    ));

    let mut invalid_media_type = resource("data:,ok");
    invalid_media_type.headers.push((
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static("not-a-media-type"),
    ));
    assert!(matches!(
        InternalResponse::from_scheme_resource(invalid_media_type, &request, FetchScheme::Data),
        Err(crate::RequestError::Provider {
            provider: "data",
            kind: crate::ProviderErrorKind::MalformedMetadata
        })
    ));

    let mut invalid_status = resource("data:,ok");
    invalid_status.status = 99;
    assert!(matches!(
        InternalResponse::from_scheme_resource(invalid_status, &request, FetchScheme::Data),
        Err(crate::RequestError::InvalidResponse(_))
    ));
}

#[test]
fn scheme_resource_preserves_bytes_streams_and_explicit_null_bodies() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let request = Request::get("data:,ok");

    let mut bytes = resource("data:,ok");
    bytes.body = SchemeBody::Bytes(b"bytes".to_vec());
    let bytes = InternalResponse::from_scheme_resource(bytes, &request, FetchScheme::Data)
        .unwrap()
        .expose();
    assert_eq!(runtime.block_on(bytes.body.bytes()).unwrap(), b"bytes");

    let mut stream = resource("data:,ok");
    stream.body = SchemeBody::Stream(Box::pin(futures_util::stream::iter(vec![
        Ok(b"stream-".to_vec()),
        Ok(b"body".to_vec()),
    ])));
    let stream = InternalResponse::from_scheme_resource(stream, &request, FetchScheme::Data)
        .unwrap()
        .expose();
    assert_eq!(
        runtime.block_on(stream.body.bytes()).unwrap(),
        b"stream-body"
    );

    let mut null = resource("data:,ok");
    null.body = SchemeBody::Null;
    let null = InternalResponse::from_scheme_resource(null, &request, FetchScheme::Data)
        .unwrap()
        .expose();
    assert!(null.body.is_null());
    assert!(runtime.block_on(null.body.bytes()).unwrap().is_empty());
}

#[test]
fn scheme_resources_synthesize_lengths_for_known_representations() {
    let request = Request::get("data:,ok");

    let mut bytes = resource("data:,ok");
    bytes.body = SchemeBody::Bytes(b"bytes".to_vec());
    let bytes = InternalResponse::from_scheme_resource(bytes, &request, FetchScheme::Data)
        .unwrap()
        .expose();
    assert_eq!(bytes.headers.get("content-length").as_deref(), Some("5"));
    assert_eq!(bytes.body.length(), Some(5));

    let mut metadata_only = resource("data:,ok");
    metadata_only.body = SchemeBody::NullWithLength(7);
    let metadata_only =
        InternalResponse::from_scheme_resource(metadata_only, &request, FetchScheme::Data)
            .unwrap()
            .expose();
    assert_eq!(
        metadata_only.headers.get("content-length").as_deref(),
        Some("7")
    );
    assert!(metadata_only.body.is_null());

    let mut stream = resource("data:,ok");
    stream.body = SchemeBody::Stream(Box::pin(futures_util::stream::empty()));
    let stream = InternalResponse::from_scheme_resource(stream, &request, FetchScheme::Data)
        .unwrap()
        .expose();
    assert!(!stream.headers.has("content-length"));
}

#[test]
fn scheme_resource_length_preserves_explicit_metadata_and_bodyless_status_rules() {
    let request = Request::get("data:,ok");

    let mut explicit = resource("data:,ok");
    explicit.body = SchemeBody::Bytes(b"bytes".to_vec());
    explicit.headers.push((
        http::header::CONTENT_LENGTH,
        http::HeaderValue::from_static("9"),
    ));
    let explicit = InternalResponse::from_scheme_resource(explicit, &request, FetchScheme::Data)
        .unwrap()
        .expose();
    assert_eq!(explicit.headers.get("content-length").as_deref(), Some("9"));
    assert_eq!(explicit.body.length(), Some(9));

    for status in [204, 205, 304] {
        let mut bodyless = resource("data:,ok");
        bodyless.status = status;
        bodyless.body = SchemeBody::Bytes(b"bytes".to_vec());
        let bodyless =
            InternalResponse::from_scheme_resource(bodyless, &request, FetchScheme::Data)
                .unwrap()
                .expose();
        assert!(!bodyless.headers.has("content-length"));
        assert!(bodyless.body.is_null());
    }
}
