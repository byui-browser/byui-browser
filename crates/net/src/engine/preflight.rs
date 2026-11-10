//! CORS-preflight request execution and response validation.

use std::{collections::BTreeSet, time::Duration};

use http::{HeaderMap, HeaderName, HeaderValue, Method};

use crate::{
    HeaderGuard, Headers,
    api::{
        error::RequestError,
        request::{
            CacheMode, CredentialsMode, PreparedRequest, RedirectMode, Request, RequestMode,
        },
    },
    cache::PreflightPermissions,
    engine::RequestController,
};

impl RequestController {
    /// Performs and caches a CORS preflight when the current hop requires one.
    pub(crate) async fn ensure_cors_preflight(
        &self,
        request: &Request,
        url: &reqwest::Url,
    ) -> Result<(), RequestError> {
        if request.context.mode != RequestMode::Cors
            || !request.effective_origin().is_some_and(|origin| {
                crate::Origin::from_url(url).is_ok_and(|target| !origin.is_same_origin(&target))
            })
            || !request.requires_cors_preflight()?
        {
            return Ok(());
        }

        let unsafe_names = request.cors_unsafe_header_names()?;
        if self
            .inner
            .preflight_cache
            .allows(request, url, &unsafe_names)
        {
            log::debug!(
                "cors preflight cache hit: {} {}",
                request.method(),
                crate::log_url::loggable(url.as_str())
            );
            return Ok(());
        }

        let headers = preflight_headers(request, &unsafe_names)?;
        let mut preflight = request.clone_without_body();
        preflight.method = Method::OPTIONS;
        preflight.headers = Headers::with_guard(HeaderGuard::None);
        preflight.body = None;
        preflight.context.mode = RequestMode::Cors;
        preflight.context.credentials = CredentialsMode::SameOrigin;
        preflight.cache_mode = CacheMode::NoStore;
        preflight.redirect_mode = RedirectMode::Error;
        preflight.integrity = None;
        preflight.keepalive = false;

        log::debug!(
            "sending cors preflight: {} {}",
            request.method(),
            crate::log_url::loggable(url.as_str())
        );
        let response = self
            .inner
            .scheduler
            .submit(
                PreparedRequest {
                    request: preflight,
                    headers,
                    url: url.clone(),
                },
                request.transport_priority(),
            )
            .await?;

        if !response.status.is_success()
            || !self
                .inner
                .cors
                .validate_response_headers(request, &response.headers)
        {
            log::warn!(
                "cors preflight denied for {} with status {}",
                crate::log_url::loggable(url.as_str()),
                response.status
            );
            return Err(RequestError::CorsDenied);
        }

        let permissions = parse_permissions(&response.headers)?;
        validate_permissions(request, &unsafe_names, &permissions)?;
        let ttl = preflight_ttl(&response.headers, self.inner.max_preflight_cache_age);
        self.inner
            .preflight_cache
            .insert(request, url, &permissions, ttl);
        Ok(())
    }
}

/// Builds the internal header list sent by a preflight request.
fn preflight_headers(
    request: &Request,
    unsafe_names: &[HeaderName],
) -> Result<Headers, RequestError> {
    let mut headers = Headers::with_guard(HeaderGuard::None);
    headers.insert_internal_header(http::header::ACCEPT, HeaderValue::from_static("*/*"))?;
    let origin = request.serialized_origin().unwrap_or_default();
    headers.insert_internal_header(
        HeaderName::from_static("origin"),
        HeaderValue::from_str(&origin).map_err(|_| RequestError::InvalidOrigin(origin))?,
    )?;
    headers.insert_internal_header(
        HeaderName::from_static("access-control-request-method"),
        HeaderValue::from_str(request.method().as_str())
            .map_err(|_| RequestError::InvalidMethod(request.method().to_string()))?,
    )?;
    if !unsafe_names.is_empty() {
        let value = unsafe_names
            .iter()
            .map(HeaderName::as_str)
            .collect::<Vec<_>>()
            .join(",");
        headers.insert_internal_header(
            HeaderName::from_static("access-control-request-headers"),
            HeaderValue::from_str(&value)
                .map_err(|_| RequestError::InvalidHeaderValue(value.clone()))?,
        )?;
    }
    Ok(headers)
}

/// Parses the method and header-name lists from a successful response.
fn parse_permissions(headers: &HeaderMap) -> Result<PreflightPermissions, RequestError> {
    Ok(PreflightPermissions {
        methods: parse_method_list(headers)?,
        header_names: parse_header_name_list(headers)?,
    })
}

/// Parses `Access-Control-Allow-Methods` as HTTP method tokens or a wildcard.
fn parse_method_list(headers: &HeaderMap) -> Result<Vec<String>, RequestError> {
    parse_comma_list(headers, "access-control-allow-methods", |token| {
        if token == "*" {
            Some(token.to_owned())
        } else {
            Method::from_bytes(token.as_bytes())
                .ok()
                .map(|method| method.as_str().to_owned())
        }
    })
}

/// Parses `Access-Control-Allow-Headers` as field names or a wildcard.
fn parse_header_name_list(headers: &HeaderMap) -> Result<Vec<String>, RequestError> {
    parse_comma_list(headers, "access-control-allow-headers", |token| {
        if token == "*" {
            Some(token.to_owned())
        } else {
            HeaderName::from_bytes(token.as_bytes())
                .ok()
                .map(|name| name.as_str().to_owned())
        }
    })
}

/// Parses repeated comma-list header fields into a sorted unique list.
fn parse_comma_list<F>(
    headers: &HeaderMap,
    name: &'static str,
    mut parse: F,
) -> Result<Vec<String>, RequestError>
where
    F: FnMut(&str) -> Option<String>,
{
    let mut output = BTreeSet::new();
    for value in headers.get_all(name) {
        let value = value.to_str().map_err(|_| RequestError::CorsDenied)?;
        if value.trim().is_empty() {
            continue;
        }
        for token in value.split(',') {
            let token = token.trim();
            let parsed = (!token.is_empty())
                .then(|| parse(token))
                .flatten()
                .ok_or(RequestError::CorsDenied)?;
            output.insert(parsed);
        }
    }
    Ok(output.into_iter().collect())
}

/// Ensures the response grants the original unsafe method and header names.
fn validate_permissions(
    request: &Request,
    unsafe_names: &[HeaderName],
    permissions: &PreflightPermissions,
) -> Result<(), RequestError> {
    let include_credentials = request.context.credentials == CredentialsMode::Include;
    let method_is_safe = matches!(
        request.method(),
        &Method::GET | &Method::HEAD | &Method::POST
    );
    let method_allowed = method_is_safe
        || permissions
            .methods
            .iter()
            .any(|method| method == request.method().as_str())
        || (!include_credentials && permissions.methods.iter().any(|method| method == "*"));
    let headers_allowed = unsafe_names.iter().all(|name| {
        permissions
            .header_names
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(name.as_str()))
            || (!include_credentials
                && name != http::header::AUTHORIZATION
                && permissions
                    .header_names
                    .iter()
                    .any(|allowed| allowed == "*"))
    });
    if method_allowed && headers_allowed {
        Ok(())
    } else {
        Err(RequestError::CorsDenied)
    }
}

/// Reads the response lifetime, applying Fetch's five-second default and cap.
fn preflight_ttl(headers: &HeaderMap, maximum: Duration) -> Duration {
    let mut values = headers.get_all("access-control-max-age").iter();
    let parsed = values
        .next()
        .and_then(|value| value.to_str().ok())
        .filter(|_| values.next().is_none())
        .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(5));
    parsed.min(maximum)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_lists_are_rejected_and_max_age_is_bounded() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "access-control-allow-methods",
            HeaderValue::from_static("PUT, not a method"),
        );
        assert!(matches!(
            parse_permissions(&headers),
            Err(RequestError::CorsDenied)
        ));

        headers.clear();
        headers.insert("access-control-max-age", HeaderValue::from_static("999999"));
        assert_eq!(
            preflight_ttl(&headers, Duration::from_secs(120)),
            Duration::from_secs(120)
        );
        headers.insert(
            "access-control-max-age",
            HeaderValue::from_static("invalid"),
        );
        assert_eq!(
            preflight_ttl(&headers, Duration::from_secs(120)),
            Duration::from_secs(5)
        );
    }
}
