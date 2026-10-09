//! Response types returned by the networking client.

use std::{
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use reqwest::{StatusCode, header::HeaderMap};

use crate::cancellation::AbortSignal;
use crate::error::RequestError;
use crate::request::{HeaderList, Origin, RequestMode};

type BoxedResponseStream = Pin<Box<dyn Stream<Item = Result<Bytes, RequestError>> + Send>>;

/// A stream of response body chunks.
///
/// Each item represents one transport-provided chunk. The stream applies
/// backpressure to the underlying HTTP response and may yield a transport
/// error after response headers have already been delivered.
pub struct ResponseBody {
    inner: BoxedResponseStream,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

impl std::fmt::Debug for ResponseBody {
    /// Formats the stream without attempting to consume it.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResponseBody")
            .finish_non_exhaustive()
    }
}

impl Stream for ResponseBody {
    type Item = Result<Bytes, RequestError>;

    /// Polls the next response body chunk.
    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(context)
    }
}

impl ResponseBody {
    /// Wraps an asynchronous byte stream as a response body.
    ///
    /// The stream is boxed so transports can provide different concrete stream
    /// types while exposing one response API. It must be `Send` because the
    /// networking work may be polled on a different executor thread.
    pub(crate) fn from_stream<S>(stream: S) -> Self
    where
        S: Stream<Item = Result<Bytes, RequestError>> + Send + 'static,
    {
        Self {
            inner: Box::pin(stream),
            permit: None,
        }
    }

    /// Creates a one-item stream for a body that is already buffered.
    pub(crate) fn once(body: Vec<u8>) -> Self {
        Self::from_stream_with_signal(
            futures_util::stream::once(async move { Ok(Bytes::from(body)) }),
            AbortSignal::new(),
        )
    }

    /// Creates a null body with no stream chunks.
    pub(crate) fn empty() -> Self {
        Self::from_stream(futures_util::stream::empty())
    }

    /// Creates a buffered response stream that observes a request signal.
    pub(crate) fn once_with_signal(body: Vec<u8>, signal: AbortSignal) -> Self {
        Self::from_stream_with_signal(
            futures_util::stream::once(async move { Ok(Bytes::from(body)) }),
            signal,
        )
    }

    /// Wraps a byte stream so aborting its signal terminates it with an error.
    pub(crate) fn from_stream_with_signal<S>(stream: S, signal: AbortSignal) -> Self
    where
        S: Stream<Item = Result<Bytes, RequestError>> + Send + 'static,
    {
        let stream = futures_util::stream::unfold(
            (Some(Box::pin(stream)), signal),
            |(stream, signal)| async move {
                let mut stream = stream?;
                if signal.is_aborted() {
                    return Some((Err(RequestError::Aborted), (None, signal)));
                }

                tokio::select! {
                    _ = signal.cancelled() => {
                        Some((Err(RequestError::Aborted), (None, signal)))
                    }
                    item = stream.next() => {
                        item.map(|item| (item, (Some(stream), signal)))
                    }
                }
            },
        );
        Self::from_stream(stream)
    }

    /// Separates the body stream from the permit that keeps a request admitted.
    ///
    /// This is used when the controller wraps the stream to capture completed
    /// responses for caching. The permit must be reattached to the replacement
    /// body so concurrency remains bounded until consumption finishes.
    pub(crate) fn into_parts(
        self,
    ) -> (
        BoxedResponseStream,
        Option<tokio::sync::OwnedSemaphorePermit>,
    ) {
        (self.inner, self.permit)
    }

    /// Holds a scheduler permit until this body is dropped.
    ///
    /// A streaming request is considered in flight while its body can still
    /// produce bytes, so releasing the permit at header receipt would allow
    /// more active network work than the configured limit permits.
    pub(crate) fn attach_permit(&mut self, permit: tokio::sync::OwnedSemaphorePermit) {
        self.permit = Some(permit);
    }
}

/// The visibility class selected by Fetch before exposing a response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponseType {
    /// A same-origin or navigation response with readable metadata and body.
    Basic,
    /// A CORS-approved response with only CORS-exposed headers.
    Cors,
    /// A no-CORS response with hidden status, URL, headers, and body.
    Opaque,
    /// A manually handled redirect with hidden metadata and body.
    /// This variant is reserved until controller-owned redirects are implemented.
    OpaqueRedirect,
    /// A failed fetch with hidden metadata and body.
    /// This variant is reserved; current fetch failures return `RequestError`.
    Error,
}

/// Complete response state retained inside the Fetch engine.
///
/// Policy and cookie processing use this state before a filtered public view
/// is built. The body owns its transport stream and scheduler permit.
pub(crate) struct InternalResponse {
    pub(crate) status: StatusCode,
    pub(crate) status_text: String,
    pub(crate) headers: HeaderMap,
    pub(crate) url_list: Vec<String>,
    pub(crate) redirect_count: usize,
    pub(crate) origin: Option<Origin>,
    pub(crate) request_origin: Option<Origin>,
    pub(crate) request_mode: RequestMode,
    pub(crate) response_type: ResponseType,
    pub(crate) body: ResponseBody,
    pub(crate) body_is_null: bool,
    pub(crate) from_cache: bool,
    pub(crate) cookie_headers_processed: bool,
}

impl InternalResponse {
    pub(crate) fn expose(self) -> StreamingResponse {
        let hidden = matches!(
            self.response_type,
            ResponseType::Opaque | ResponseType::OpaqueRedirect | ResponseType::Error
        );
        let headers = if hidden {
            HeaderList::with_guard(crate::HeaderGuard::Immutable)
        } else {
            HeaderList::exposed_response(&self.headers, self.response_type == ResponseType::Cors)
        };
        StreamingResponse {
            response_type: self.response_type,
            status: if hidden { 0 } else { self.status.as_u16() },
            status_text: if hidden {
                String::new()
            } else {
                self.status_text
            },
            headers,
            url: if hidden {
                String::new()
            } else {
                self.url_list.last().cloned().unwrap_or_default()
            },
            redirected: !hidden && self.redirect_count > 0,
            body: if hidden || self.body_is_null {
                ResponseBody::empty()
            } else {
                self.body
            },
            from_cache: !hidden && self.from_cache,
        }
    }
}

/// A fully buffered, already filtered Fetch response.
#[derive(Clone, Debug)]
pub struct Response {
    /// Fetch response visibility class.
    pub response_type: ResponseType,
    /// Exposed HTTP status code, or zero for filtered responses.
    pub status: u16,
    /// Canonical HTTP reason phrase, empty for filtered responses.
    /// A custom wire reason phrase is not retained by the current transport.
    pub status_text: String,
    /// Immutable exposed response headers, excluding cookie headers.
    pub headers: HeaderList,
    /// Exposed final URL, empty for filtered responses.
    pub url: String,
    /// Whether the request followed at least one redirect.
    pub redirected: bool,
    /// Exposed body bytes. Filtered and null bodies are empty.
    pub body: Vec<u8>,
    /// Whether this readable response came from the process-local cache.
    /// Filtered opaque responses always report `false` to avoid metadata leaks.
    pub from_cache: bool,
}

/// Response metadata and a lazily consumed HTTP response body.
///
/// Headers are available as soon as the server response is received. The body
/// remains attached to the network transport and is consumed by polling
/// [`StreamingResponse::body`].
pub struct StreamingResponse {
    /// Fetch response visibility class.
    pub response_type: ResponseType,
    /// Exposed HTTP status code, or zero for filtered responses.
    pub status: u16,
    /// Canonical HTTP reason phrase, empty for filtered responses.
    /// A custom wire reason phrase is not retained by the current transport.
    pub status_text: String,
    /// Immutable exposed response headers, excluding cookie headers.
    pub headers: HeaderList,
    /// Exposed final URL, empty for filtered responses.
    pub url: String,
    /// Whether the request followed at least one redirect.
    pub redirected: bool,
    /// Filtered, abort-aware body chunks. A dropped body releases its permit.
    pub body: ResponseBody,
    /// Whether this readable response came from the process-local cache.
    /// Filtered opaque responses always report `false` to avoid metadata leaks.
    pub from_cache: bool,
}

impl std::fmt::Debug for StreamingResponse {
    /// Formats response metadata without consuming the body.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StreamingResponse")
            .field("status", &self.status)
            .field("headers", &self.headers)
            .field("url", &self.url)
            .field("body", &self.body)
            .field("from_cache", &self.from_cache)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use futures_util::{StreamExt, stream};

    use super::*;
    use crate::AbortController;

    #[test]
    fn a_pending_response_stream_ends_with_aborted_error() {
        let runtime = tokio::runtime::Runtime::new().expect("Tokio runtime should initialize");
        let controller = AbortController::new();
        let mut body = ResponseBody::from_stream_with_signal(
            stream::pending::<Result<Bytes, RequestError>>(),
            controller.signal(),
        );
        controller.abort();

        let item = runtime
            .block_on(body.next())
            .expect("abort should produce an item");

        assert!(matches!(item, Err(RequestError::Aborted)));
    }

    #[test]
    fn basic_view_hides_cookie_headers_before_exposure() {
        let mut headers = HeaderMap::new();
        headers.insert("set-cookie", "session=secret".parse().unwrap());
        headers.insert("x-visible", "yes".parse().unwrap());
        let internal = InternalResponse {
            status: StatusCode::OK,
            status_text: "OK".into(),
            headers,
            url_list: vec!["https://example.test/".into()],
            redirect_count: 0,
            origin: None,
            request_origin: None,
            request_mode: RequestMode::Cors,
            response_type: ResponseType::Basic,
            body: ResponseBody::once(b"body".to_vec()),
            body_is_null: false,
            from_cache: false,
            cookie_headers_processed: true,
        };
        assert!(internal.headers.contains_key("set-cookie"));
        let exposed = internal.expose();
        assert_eq!(exposed.headers.get("x-visible").unwrap(), "yes");
        assert!(!exposed.headers.has("set-cookie"));
        assert_eq!(exposed.headers.guard(), crate::HeaderGuard::Immutable);
    }

    #[test]
    fn opaque_and_null_views_cannot_yield_transport_bytes() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        for (response_type, body_is_null) in
            [(ResponseType::Opaque, false), (ResponseType::Basic, true)]
        {
            let internal = InternalResponse {
                status: StatusCode::NO_CONTENT,
                status_text: "No Content".into(),
                headers: HeaderMap::new(),
                url_list: vec!["https://example.test/".into()],
                redirect_count: 0,
                origin: None,
                request_origin: None,
                request_mode: RequestMode::Cors,
                response_type,
                body: ResponseBody::once(b"forbidden".to_vec()),
                body_is_null,
                from_cache: response_type == ResponseType::Opaque,
                cookie_headers_processed: false,
            };
            let mut exposed = internal.expose();
            if response_type == ResponseType::Opaque {
                assert_eq!(exposed.status, 0);
                assert!(exposed.url.is_empty());
                assert!(exposed.headers.is_empty());
                assert!(!exposed.from_cache);
            }
            assert!(
                runtime
                    .block_on(async { exposed.body.next().await })
                    .is_none()
            );
        }
    }
}
