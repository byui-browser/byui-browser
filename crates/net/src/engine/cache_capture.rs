//! Cache insertion after successful streamed-body consumption.

use futures_util::{StreamExt, stream};

use crate::{
    api::{body::Body, error::RequestError, request::Request, response::InternalResponse},
    cache::{ResponseCache, StoredResponse},
};

/// State retained while a successful response body is copied into the cache.
struct CacheCapture {
    /// Cache receiving the body after successful end-of-stream.
    cache: ResponseCache,
    /// Original request used to compute the cache key.
    request: Request,
    /// Parsed request URL used to compute the cache key.
    request_url: reqwest::Url,
    /// Response metadata retained while the body is consumed.
    status: reqwest::StatusCode,
    headers: reqwest::header::HeaderMap,
    url: String,
    cookie_headers_processed: bool,
    /// Maximum bytes retained for this cache entry.
    max_body_size: usize,
}

/// Wraps a response body so completed network responses are copied into cache.
///
/// Bytes are forwarded immediately to the caller while a second copy is
/// collected. Any body error abandons the capture, and only a clean end of
/// stream inserts the complete response.
pub(crate) fn with_cache_capture(
    mut response: InternalResponse,
    cache: ResponseCache,
    request: Request,
    request_url: reqwest::Url,
    max_body_size: usize,
) -> Result<InternalResponse, RequestError> {
    let status = response.status;
    let headers = response.headers.clone();
    let url = response.url_list.last().cloned().unwrap_or_default();
    let length = response.body.length();
    let (body, permit) = response.body.try_into_parts()?;
    let capture = CacheCapture {
        cache,
        request,
        request_url,
        status,
        headers,
        url,
        cookie_headers_processed: response.cookie_headers_processed,
        max_body_size,
    };
    let body = stream::unfold(
        (body, Vec::new(), Some(capture)),
        |(mut body, mut collected, capture)| async move {
            match body.next().await {
                Some(Ok(chunk)) => {
                    let capture = capture.filter(|capture| {
                        collected.len().saturating_add(chunk.len()) <= capture.max_body_size
                    });
                    if capture.is_some() {
                        collected.extend_from_slice(&chunk);
                    } else {
                        collected.clear();
                    }
                    Some((Ok(chunk), (body, collected, capture)))
                }
                Some(Err(error)) => {
                    if capture.is_some() {
                        log::debug!("cache capture abandoned after body error: {error}");
                    }
                    Some((Err(error), (body, collected, None)))
                }
                None => {
                    if let Some(capture) = capture {
                        log::trace!(
                            "body fully consumed, storing {} bytes for {}",
                            collected.len(),
                            capture.url
                        );
                        capture.cache.insert(
                            &capture.request,
                            &capture.request_url,
                            &StoredResponse {
                                status: capture.status,
                                headers: capture.headers,
                                url: capture.url,
                                body: collected,
                                cookie_headers_processed: capture.cookie_headers_processed,
                            },
                        );
                    }
                    None
                }
            }
        },
    );
    let mut body = Body::from_stream(body);
    body.set_response_metadata(&response.headers, length);
    if let Some(permit) = permit {
        body.attach_permit(permit);
    }
    response.body = body;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use futures_util::{StreamExt, stream};
    use reqwest::{
        StatusCode,
        header::{CACHE_CONTROL, HeaderMap, HeaderValue},
    };

    use super::*;
    use crate::api::request::FetchContext;

    #[test]
    fn disturbed_source_cannot_become_an_empty_cached_response() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let url = reqwest::Url::parse("https://example.test/resource").unwrap();
        let request = Request::get(url.as_str());
        let cache = ResponseCache::default();
        let mut headers = HeaderMap::new();
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("max-age=60"));
        let mut body = Body::from_stream(stream::iter([Ok(Bytes::from_static(b"first"))]));
        let _ = runtime.block_on(body.next());
        let response = InternalResponse::from_http_parts(
            StatusCode::OK,
            headers,
            url.to_string(),
            &FetchContext::default(),
            body,
            false,
            false,
        );

        assert!(matches!(
            with_cache_capture(
                response,
                cache.clone(),
                request.try_clone().unwrap(),
                url.clone(),
                1024
            ),
            Err(RequestError::BodyAlreadyConsumed)
        ));
        assert!(cache.get(&request, &url).is_none());
    }
}
