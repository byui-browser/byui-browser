//! Background cache insertion after successful streamed-body capture.

use bytes::Bytes;
use futures_util::StreamExt;

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

/// Starts bounded background capture for a response body.
///
/// The caller receives one tee branch immediately while a background task
/// consumes the other. The tee has one queued chunk per branch, so capture
/// cannot accumulate an unbounded backlog when the caller is slow. Dropping
/// the caller's branch lets capture finish independently. Any body error,
/// abort, or size-limit overflow abandons the entry; only a clean, bounded
/// end of stream inserts the complete response.
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
    let capture_body = response.body.tee()?;
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
    tokio::spawn(capture_body_to_cache(capture_body, capture));
    Ok(response)
}

/// Collects one tee branch and stores it only after clean completion.
async fn capture_body_to_cache(mut body: Body, capture: CacheCapture) {
    let mut collected = Vec::new();
    let mut oversized = false;

    while let Some(chunk) = body.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                log::debug!("cache capture abandoned after body error: {error}");
                return;
            }
        };
        if !oversized {
            if collected.len().saturating_add(chunk.len()) <= capture.max_body_size {
                collected.extend_from_slice(&chunk);
            } else {
                oversized = true;
                collected.clear();
                log::debug!(
                    "cache capture abandoned after exceeding {} bytes for {}",
                    capture.max_body_size,
                    capture.url
                );
            }
        }
    }

    if oversized {
        return;
    }
    log::trace!(
        "body captured, storing {} bytes for {}",
        collected.len(),
        capture.url
    );
    capture.cache.insert(
        &capture.request,
        &capture.request_url,
        StoredResponse {
            status: capture.status,
            headers: capture.headers,
            url: capture.url,
            body: Bytes::from(collected),
            cookie_headers_processed: capture.cookie_headers_processed,
        },
    );
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
    use crate::{AbortController, api::request::FetchContext};

    #[test]
    fn disturbed_source_cannot_become_an_empty_cached_response() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
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

        let _guard = runtime.enter();
        assert!(matches!(
            with_cache_capture(
                response,
                cache.clone(),
                request.try_clone().unwrap(),
                url.clone(),
                1024
            ),
            Err(RequestError::BodyNotCloneable)
        ));
        assert!(cache.lookup(&request, &url).is_none());
    }

    #[test]
    fn dropped_caller_body_is_captured_in_the_background() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let url = reqwest::Url::parse("https://example.test/resource").unwrap();
        let request = Request::get(url.as_str());
        let cache = ResponseCache::default();
        let mut headers = HeaderMap::new();
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("max-age=60"));
        let response = InternalResponse::from_http_parts(
            StatusCode::OK,
            headers,
            url.to_string(),
            &FetchContext::default(),
            Body::from_stream(stream::iter([Ok(Bytes::from_static(b"captured"))])),
            false,
            false,
        );

        runtime.block_on(async {
            let response = with_cache_capture(
                response,
                cache.clone(),
                request.try_clone().unwrap(),
                url.clone(),
                1024,
            )
            .unwrap();
            drop(response);
            for _ in 0..4_096 {
                if cache.lookup(&request, &url).is_some() {
                    return;
                }
                tokio::task::yield_now().await;
            }
        });
        assert!(cache.lookup(&request, &url).is_some());
    }

    #[test]
    fn oversized_body_is_not_cached_after_background_capture() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let url = reqwest::Url::parse("https://example.test/resource").unwrap();
        let request = Request::get(url.as_str());
        let cache = ResponseCache::default();
        let mut headers = HeaderMap::new();
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("max-age=60"));
        let response = InternalResponse::from_http_parts(
            StatusCode::OK,
            headers,
            url.to_string(),
            &FetchContext::default(),
            Body::from_stream(stream::iter([Ok(Bytes::from_static(b"too large"))])),
            false,
            false,
        );

        let response = runtime.block_on(async {
            with_cache_capture(
                response,
                cache.clone(),
                request.try_clone().unwrap(),
                url.clone(),
                3,
            )
            .unwrap()
        });
        drop(response);
        runtime.block_on(async {
            for _ in 0..4_096 {
                tokio::task::yield_now().await;
            }
        });
        assert!(cache.lookup(&request, &url).is_none());
    }

    #[test]
    fn failed_body_is_not_cached_after_background_capture() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let url = reqwest::Url::parse("https://example.test/resource").unwrap();
        let request = Request::get(url.as_str());
        let cache = ResponseCache::default();
        let mut headers = HeaderMap::new();
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("max-age=60"));
        let response = InternalResponse::from_http_parts(
            StatusCode::OK,
            headers,
            url.to_string(),
            &FetchContext::default(),
            Body::from_stream(stream::iter([Err(RequestError::Transport(
                "failed capture".into(),
            ))])),
            false,
            false,
        );

        runtime.block_on(async {
            let response = with_cache_capture(
                response,
                cache.clone(),
                request.try_clone().unwrap(),
                url.clone(),
                1024,
            )
            .unwrap();
            drop(response);
            for _ in 0..4_096 {
                tokio::task::yield_now().await;
            }
        });
        assert!(cache.lookup(&request, &url).is_none());
    }

    #[test]
    fn aborted_body_is_not_cached_after_background_capture() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let abort = AbortController::new();
        let url = reqwest::Url::parse("https://example.test/resource").unwrap();
        let request = Request::get(url.as_str());
        let cache = ResponseCache::default();
        let mut headers = HeaderMap::new();
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("max-age=60"));
        let response = InternalResponse::from_http_parts(
            StatusCode::OK,
            headers,
            url.to_string(),
            &FetchContext::default(),
            Body::from_stream_with_signal(stream::pending(), abort.signal()),
            false,
            false,
        );

        runtime.block_on(async {
            let response = with_cache_capture(
                response,
                cache.clone(),
                request.try_clone().unwrap(),
                url.clone(),
                1024,
            )
            .unwrap();
            abort.abort();
            drop(response);
            for _ in 0..4_096 {
                tokio::task::yield_now().await;
            }
        });
        assert!(cache.lookup(&request, &url).is_none());
    }
}
