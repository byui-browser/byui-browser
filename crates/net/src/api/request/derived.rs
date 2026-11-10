use http::{
    Method,
    header::{CONTENT_TYPE, HeaderName, HeaderValue},
};
use url::Url;

use crate::api::{
    Headers,
    body::Body,
    error::RequestError,
    headers::{cors_unsafe_request_header_names, is_cors_safelisted_header},
};

use super::{
    CredentialsMode, Origin, RedirectTaint, Referrer, ReferrerPolicy, Request, RequestDuplex,
    RequestMode, RequestPriority, urls_are_same_origin, urls_are_same_site,
};

impl Request {
    /// Returns whether this request method may use the response cache.
    pub(crate) fn is_cacheable_method(&self) -> bool {
        matches!(self.method, Method::GET | Method::HEAD)
    }
    /// Returns whether the credentials mode permits credentials for this URL.
    pub(crate) fn credentials_allowed(&self, url: &Url) -> bool {
        match self.context.credentials {
            CredentialsMode::Omit => false,
            CredentialsMode::Include => true,
            CredentialsMode::SameOrigin => self.has_same_origin_target(url),
        }
    }

    /// Reports whether the current target remains in a same-origin request chain.
    pub(crate) fn has_same_origin_target(&self, url: &Url) -> bool {
        self.redirect_taint() == RedirectTaint::SameOrigin
            && self.effective_origin().is_some_and(|origin| {
                Origin::from_url(url).is_ok_and(|target| origin.is_same_origin(&target))
            })
    }

    /// Returns the request origin after any redirect-time origin transitions.
    pub(crate) fn effective_origin(&self) -> Option<&Origin> {
        self.request_origin
            .as_ref()
            .or(self.context.environment.origin.as_ref())
    }

    /// Computes the redirect taint from the complete request URL list.
    pub(crate) fn redirect_taint(&self) -> RedirectTaint {
        let Some(origin) = self.effective_origin() else {
            return RedirectTaint::SameOrigin;
        };
        let mut taint = RedirectTaint::SameOrigin;
        for pair in self.url_list.windows(2) {
            let previous = &pair[0];
            let current = &pair[1];
            if !urls_are_same_site(current, previous) && !origin.is_same_site_url(previous) {
                return RedirectTaint::CrossSite;
            }
            let origin_matches_previous = Origin::from_url(previous)
                .is_ok_and(|previous_origin| origin.is_same_origin(&previous_origin));
            if !urls_are_same_origin(current, previous) && !origin_matches_previous {
                taint = RedirectTaint::SameSite;
            }
        }
        taint
    }

    /// Serializes the request origin, accounting for redirect taint.
    pub(crate) fn serialized_origin(&self) -> Option<String> {
        self.effective_origin().map(|origin| {
            if self.redirect_taint() != RedirectTaint::SameOrigin {
                "null".to_owned()
            } else {
                origin.as_str()
            }
        })
    }
    /// Returns the effective request-header names requiring CORS approval.
    ///
    /// Body-derived content type participates, while user-agent-generated
    /// content length and `Origin` fields do not.
    pub(crate) fn cors_unsafe_header_names(&self) -> Result<Vec<HeaderName>, RequestError> {
        let mut headers = self.headers.clone();
        if let Some(body) = &self.body
            && let Some(content_type) = body.content_type()
            && !headers.has(CONTENT_TYPE.as_str())
        {
            headers.insert_internal_header(CONTENT_TYPE, content_type)?;
        }
        Ok(cors_unsafe_request_header_names(&headers))
    }

    /// Reports whether method or effective headers require a CORS preflight.
    pub(crate) fn requires_cors_preflight(&self) -> Result<bool, RequestError> {
        Ok(
            !matches!(self.method, Method::GET | Method::HEAD | Method::POST)
                || !self.cors_unsafe_header_names()?.is_empty(),
        )
    }
    /// Returns the transport priority corresponding to the Fetch priority.
    pub(crate) fn transport_priority(&self) -> crate::scheduling::RequestPriority {
        match self.priority {
            RequestPriority::Low => crate::scheduling::RequestPriority::Low,
            RequestPriority::Auto => crate::scheduling::RequestPriority::Normal,
            RequestPriority::High => crate::scheduling::RequestPriority::High,
        }
    }
    /// Builds the internal transport header channel without changing caller headers.
    pub(crate) fn fetch_headers(&self, target: &Url) -> Result<Headers, RequestError> {
        let mut headers = self.headers.clone();
        if let Some(body) = &self.body {
            body.apply_metadata(&mut headers)?;
        }
        if let Some(origin) = self.origin_header_value(target) {
            let value = HeaderValue::try_from(&origin)
                .map_err(|_| RequestError::InvalidOrigin(origin.clone()))?;
            headers.insert_internal_header(HeaderName::from_static("origin"), value)?;
        }
        Ok(headers)
    }

    /// Computes the user-agent-owned `Origin` value for one HTTP target.
    fn origin_header_value(&self, target: &Url) -> Option<String> {
        let mut serialized = self.serialized_origin()?;
        let cors_tainted =
            self.context.mode == RequestMode::Cors && !self.has_same_origin_target(target);
        if cors_tainted {
            return Some(serialized);
        }
        if matches!(self.method, Method::GET | Method::HEAD) {
            return None;
        }
        if self.context.mode != RequestMode::Cors {
            let policy = match self.referrer_policy {
                ReferrerPolicy::Empty => ReferrerPolicy::StrictOriginWhenCrossOrigin,
                policy => policy,
            };
            let downgrade = self
                .effective_origin()
                .is_some_and(|origin| origin.scheme() == Some("https"))
                && target.scheme() != "https";
            let cross_origin = !self.has_same_origin_target(target);
            if policy == ReferrerPolicy::NoReferrer
                || (matches!(
                    policy,
                    ReferrerPolicy::NoReferrerWhenDowngrade
                        | ReferrerPolicy::StrictOrigin
                        | ReferrerPolicy::StrictOriginWhenCrossOrigin
                ) && downgrade)
                || (policy == ReferrerPolicy::SameOrigin && cross_origin)
            {
                serialized = "null".to_owned();
            }
        }
        Some(serialized)
    }
    /// Computes the `Referer` value for a request target.
    pub(crate) fn referrer_value(&self, target: &Url) -> Option<String> {
        let source = match &self.referrer {
            Referrer::NoReferrer => return None,
            Referrer::Client => self.context.environment.referrer.as_ref()?,
            Referrer::Url(source) => source,
        };
        let same_origin = source.origin() == target.origin();
        let downgrade = source.scheme() == "https" && target.scheme() == "http";
        let source_url = source.as_str().to_owned();
        let source_origin = source.origin().ascii_serialization();
        let policy = match self.referrer_policy {
            ReferrerPolicy::Empty => ReferrerPolicy::StrictOriginWhenCrossOrigin,
            policy => policy,
        };
        match policy {
            ReferrerPolicy::UnsafeUrl => Some(source_url),
            ReferrerPolicy::NoReferrer => None,
            ReferrerPolicy::NoReferrerWhenDowngrade if downgrade => None,
            ReferrerPolicy::NoReferrerWhenDowngrade => Some(source_url),
            ReferrerPolicy::Origin => Some(source_origin),
            ReferrerPolicy::OriginWhenCrossOrigin if same_origin => Some(source_url),
            ReferrerPolicy::OriginWhenCrossOrigin => Some(source_origin),
            ReferrerPolicy::SameOrigin if same_origin => Some(source_url),
            ReferrerPolicy::SameOrigin => None,
            ReferrerPolicy::StrictOrigin if downgrade => None,
            ReferrerPolicy::StrictOrigin => Some(source_origin),
            ReferrerPolicy::StrictOriginWhenCrossOrigin if downgrade => None,
            ReferrerPolicy::StrictOriginWhenCrossOrigin if same_origin => Some(source_url),
            ReferrerPolicy::StrictOriginWhenCrossOrigin => Some(source_origin),
            ReferrerPolicy::Empty => Some(source_origin),
        }
    }

    /// Applies the active policy so later redirects cannot regain referrer detail.
    pub(crate) fn reduce_referrer_for_target(&mut self, target: &Url) {
        self.referrer = self
            .referrer_value(target)
            .and_then(|value| Url::parse(&value).ok())
            .map(Referrer::Url)
            .unwrap_or(Referrer::NoReferrer);
    }
    /// Validates method, no-CORS, and keepalive constraints for the body.
    pub(crate) fn validate_body_state(
        &self,
        max_keepalive_body_size: u64,
    ) -> Result<(), RequestError> {
        if self.body.is_some() && matches!(self.method, Method::GET | Method::HEAD) {
            return Err(RequestError::BodyNotAllowed(self.method.to_string()));
        }
        if self.body.as_ref().is_some_and(Body::is_streaming_source)
            && self.duplex != Some(RequestDuplex::Half)
        {
            return Err(RequestError::StreamingBodyRequiresDuplex);
        }
        if self.context.mode == RequestMode::NoCors
            && let Some(body) = &self.body
            && let Some(content_type) = body.content_type()
            && !is_cors_safelisted_header(&CONTENT_TYPE, &content_type)
        {
            return Err(RequestError::ForbiddenHeader(
                CONTENT_TYPE.as_str().to_owned(),
            ));
        }
        if self.keepalive {
            let body_size = match &self.body {
                None => Some(0),
                Some(body) => body.length(),
            };
            if body_size.is_none_or(|size| size > max_keepalive_body_size) {
                return Err(RequestError::KeepaliveBodyTooLarge {
                    limit: max_keepalive_body_size,
                });
            }
        }
        Ok(())
    }
}
