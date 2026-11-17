use http::Method;
use url::Url;

use crate::api::{Headers, body::Body, cancellation::AbortSignal};

use super::{CredentialsMode, FetchEnvironment, Request, RequestMode};

impl Request {
    /// Returns the original URL string supplied for this request.
    pub fn url(&self) -> &str {
        &self.url
    }
    /// Returns the URL currently being processed by the controller.
    ///
    /// Before preparation this is the original URL supplied by the caller.
    /// During a redirect chain it is the URL of the current hop. Browser
    /// services that make per-hop decisions should use this rather than
    /// [`Request::url`], which intentionally preserves the original input.
    pub fn active_url(&self) -> &str {
        self.current_url
            .as_ref()
            .map(Url::as_str)
            .unwrap_or(&self.url)
    }
    /// Returns this request's process-local identity.
    ///
    /// The identifier is stable across clones and is intended for
    /// browser-owned request bookkeeping. It does not itself authorize any
    /// operation.
    pub fn id(&self) -> u64 {
        self.id
    }
    /// Returns the normalized HTTP method.
    pub fn method(&self) -> &Method {
        &self.method
    }
    /// Returns the guarded request header list.
    pub fn headers(&self) -> &Headers {
        &self.headers
    }
    /// Returns this request's optional body.
    pub fn body(&self) -> Option<&Body> {
        self.body.as_ref()
    }
    /// Returns the resolved current URL once request preparation has run.
    pub fn current_url(&self) -> Option<&Url> {
        self.current_url.as_ref()
    }
    /// Returns the configured request mode.
    pub fn mode(&self) -> RequestMode {
        self.context.mode
    }
    /// Returns the configured credentials mode.
    pub fn credentials_mode(&self) -> CredentialsMode {
        self.context.credentials
    }
    /// Returns the client environment used to derive URL and origin state.
    pub fn environment(&self) -> &FetchEnvironment {
        &self.context.environment
    }
    /// Returns all URLs visited by this request, in order.
    pub fn url_list(&self) -> &[Url] {
        &self.url_list
    }
    /// Returns the number of redirects followed so far.
    pub fn redirect_count(&self) -> usize {
        self.redirect_count
    }
    /// Returns the cancellation signal associated with this request.
    ///
    /// Scheme providers may clone the signal when provider-owned work must
    /// outlive the initial resolution future.
    pub fn signal(&self) -> &AbortSignal {
        &self.signal
    }
}
