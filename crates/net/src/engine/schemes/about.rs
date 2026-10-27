//! Engine-owned allowlisted `about:` scheme handling.

use http::{HeaderName, HeaderValue, Method};
use reqwest::Url;

use crate::{
    api::{
        error::RequestError,
        providers::{SchemeBody, SchemeResource},
        request::{Origin, Request},
        response::Response,
    },
    engine::RequestController,
    policy::FetchScheme,
};

impl RequestController {
    /// Fetches an allowlisted `about:` URL.
    ///
    /// The blank response is engine-owned and does not consult providers,
    /// cookies, the HTTP cache, or network transport. `GET` exposes an empty
    /// readable body; `HEAD` exposes an explicit null body.
    pub(crate) async fn fetch_about(
        &self,
        request: Request,
        url: Url,
    ) -> Result<Response, RequestError> {
        let Some(allowed_url) = allowed_about_url(&url) else {
            return Err(RequestError::UnsupportedFeature("about URL"));
        };
        if !matches!(request.method(), &Method::GET | &Method::HEAD) {
            return Err(RequestError::UnsupportedFeature("about scheme method"));
        }

        let resource = SchemeResource {
            url,
            status: allowed_url.status,
            status_text: Some(allowed_url.status_text.into()),
            headers: vec![
                (
                    HeaderName::from_static("content-type"),
                    HeaderValue::from_static(allowed_url.content_type),
                ),
                (
                    HeaderName::from_static("content-length"),
                    HeaderValue::from_static("0"),
                ),
            ],
            media_type: None,
            origin: Origin::opaque(),
            body: allowed_url.body(request.method()),
        };
        self.finalize_scheme_resource(&request, resource, FetchScheme::About)
    }
}

/// The engine-owned `about:` resources supported by the first-pass handler.
///
/// Add a new entry here when a new `about:` URL is intentionally supported.
/// URL matching remains exact: hierarchical URLs, queries, and fragments do
/// not match an entry.
const ABOUT_ALLOWLIST: &[AboutAllowlistEntry] = &[AboutAllowlistEntry {
    path: "blank",
    status: 200,
    status_text: "OK",
    content_type: "text/html;charset=utf-8",
    body: AboutBody::Empty,
}];

struct AboutAllowlistEntry {
    path: &'static str,
    status: u16,
    status_text: &'static str,
    content_type: &'static str,
    body: AboutBody,
}

#[derive(Clone, Copy)]
enum AboutBody {
    Empty,
}

impl AboutAllowlistEntry {
    fn body(&self, method: &Method) -> SchemeBody {
        if method == Method::HEAD {
            SchemeBody::Null
        } else {
            match self.body {
                AboutBody::Empty => SchemeBody::Bytes(Vec::new()),
            }
        }
    }
}

/// Returns the exact allowlist entry for `url`, if one exists.
fn allowed_about_url(url: &Url) -> Option<&'static AboutAllowlistEntry> {
    if url.scheme() != "about"
        || url.host_str().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }

    ABOUT_ALLOWLIST
        .iter()
        .find(|entry| url.path() == entry.path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_exact_blank_urls_are_allowlisted() {
        assert!(allowed_about_url(&Url::parse("about:blank").unwrap()).is_some());
        assert!(allowed_about_url(&Url::parse("ABOUT:blank").unwrap()).is_some());
        for value in [
            "about:srcdoc",
            "about:blank?query",
            "about:blank#fragment",
            "about://blank",
            "about:unknown",
        ] {
            assert!(
                allowed_about_url(&Url::parse(value).unwrap()).is_none(),
                "{value}"
            );
        }
    }
}
