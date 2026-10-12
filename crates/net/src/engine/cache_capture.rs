//! Cache insertion after successful streamed-body consumption.

use futures_util::{StreamExt, stream};

use crate::{
    api::{body::Body, request::Request, response::InternalResponse},
    cache::{ResponseCache, StoredResponse},
};

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
) -> InternalResponse {
    let status = response.status;
    let headers = response.headers.clone();
    let url = response.url_list.last().cloned().unwrap_or_default();
    let (body, permit) = response.body.into_parts();
    let capture = CacheCapture {
        cache,
        request,
        request_url,
        status,
        headers,
        url,
        cookie_headers_processed: response.cookie_headers_processed,
    };
    let body = stream::unfold(
        (body, Vec::new(), Some(capture)),
        |(mut body, mut collected, capture)| async move {
            match body.next().await {
                Some(Ok(chunk)) => {
                    collected.extend_from_slice(&chunk);
                    Some((Ok(chunk), (body, collected, capture)))
                }
                Some(Err(error)) => Some((Err(error), (body, collected, None))),
                None => {
                    if let Some(capture) = capture {
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
    body.set_content_type(response.headers.get(http::header::CONTENT_TYPE).cloned());
    if let Some(permit) = permit {
        body.attach_permit(permit);
    }
    response.body = body;
    response
}
