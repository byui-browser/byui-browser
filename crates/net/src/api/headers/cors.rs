use http::header::{HeaderMap, HeaderName, HeaderValue};

use crate::api::request::CredentialsMode;

use super::Headers;

#[derive(Default)]
/// Parsed `Access-Control-Expose-Headers` state used during response filtering.
pub(super) struct CorsExposure {
    wildcard: bool,
    names: Vec<HeaderName>,
}

impl CorsExposure {
    /// Parses exposed names, applying the credential-sensitive wildcard rule.
    pub(super) fn parse(headers: &HeaderMap, credentials_mode: CredentialsMode) -> Self {
        let mut names = Vec::new();
        let mut wildcard = false;
        for value in headers.get_all("access-control-expose-headers").iter() {
            let Ok(value) = value.to_str() else {
                return Self::default();
            };
            for item in value.split(',') {
                let item = item.trim();
                if item.is_empty() {
                    continue;
                }
                if item == "*" {
                    wildcard = credentials_mode != CredentialsMode::Include;
                    continue;
                }
                let Ok(name) = HeaderName::from_bytes(item.as_bytes()) else {
                    return Self::default();
                };
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        Self { wildcard, names }
    }

    /// Reports whether a response header is visible to the caller.
    pub(super) fn includes(&self, name: &HeaderName) -> bool {
        is_cors_safelisted_response_header(name) || self.wildcard || self.names.contains(name)
    }
}

/// Reports whether a response header is always exposed by CORS.
fn is_cors_safelisted_response_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "cache-control"
            | "content-language"
            | "content-length"
            | "content-type"
            | "expires"
            | "last-modified"
            | "pragma"
    )
}
/// Reports whether a request header/value pair is CORS-safelisted.
pub(crate) fn is_cors_safelisted_header(name: &HeaderName, value: &HeaderValue) -> bool {
    let raw = value.as_bytes();
    if raw.len() > 128 {
        return false;
    }
    match name.as_str() {
        "accept" => !raw.iter().copied().any(is_cors_unsafe_request_header_byte),
        "accept-language" | "content-language" => raw.iter().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(*byte, b' ' | b'*' | b',' | b'-' | b'.' | b';' | b'=')
        }),
        "content-type" => {
            !raw.iter().copied().any(is_cors_unsafe_request_header_byte)
                && value.to_str().is_ok_and(|value| {
                    let media_type = value
                        .split(';')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_ascii_lowercase();
                    matches!(
                        media_type.as_str(),
                        "application/x-www-form-urlencoded" | "multipart/form-data" | "text/plain"
                    )
                })
        }
        "range" => value.to_str().is_ok_and(|value| {
            value.strip_prefix("bytes=").is_some_and(|range| {
                let mut parts = range.split('-');
                let first = parts.next().unwrap_or("");
                let last = parts.next().unwrap_or("");
                !first.is_empty()
                    && first.bytes().all(|byte| byte.is_ascii_digit())
                    && (last.is_empty() || last.bytes().all(|byte| byte.is_ascii_digit()))
                    && parts.next().is_none()
            })
        }),
        _ => false,
    }
}

/// Returns the sorted lowercase names that require CORS preflight approval.
///
/// Individually safelisted fields become unsafe when their combined value size
/// exceeds Fetch's 1024-byte aggregate limit.
pub(crate) fn cors_unsafe_request_header_names(headers: &Headers) -> Vec<HeaderName> {
    let mut unsafe_names = Vec::new();
    let mut potentially_unsafe_names = Vec::new();
    let mut safelist_value_size = 0usize;

    for (name, value) in headers.iter_raw() {
        if is_cors_safelisted_header(name, value) {
            potentially_unsafe_names.push(name.clone());
            safelist_value_size = safelist_value_size.saturating_add(value.as_bytes().len());
        } else {
            unsafe_names.push(name.clone());
        }
    }
    if safelist_value_size > 1024 {
        unsafe_names.extend(potentially_unsafe_names);
    }
    unsafe_names.sort_unstable_by(|left, right| left.as_str().cmp(right.as_str()));
    unsafe_names.dedup();
    unsafe_names
}

/// Reports whether a byte is forbidden in relevant CORS-safelisted values.
fn is_cors_unsafe_request_header_byte(byte: u8) -> bool {
    (byte < 0x20 && byte != b'\t')
        || matches!(
            byte,
            b'"' | b'('
                | b')'
                | b':'
                | b'<'
                | b'>'
                | b'?'
                | b'@'
                | b'['
                | b'\\'
                | b']'
                | b'{'
                | b'}'
                | 0x7f
        )
}
