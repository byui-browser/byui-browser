use http::header::{HeaderName, HeaderValue};

use crate::api::error::RequestError;

/// Parses a header name and maps invalid syntax to the public request error.
pub(super) fn parse_name(name: &str) -> Result<HeaderName, RequestError> {
    HeaderName::from_bytes(name.as_bytes())
        .map_err(|_| RequestError::InvalidHeaderName(name.to_owned()))
}

/// Parses a header value and associates failures with its header name.
pub(super) fn parse_value(name: &str, value: &str) -> Result<HeaderValue, RequestError> {
    HeaderValue::try_from(value).map_err(|_| RequestError::InvalidHeaderValue(name.to_owned()))
}

/// Trims optional surrounding horizontal whitespace from a header value.
pub(super) fn normalize_value(value: HeaderValue) -> HeaderValue {
    let bytes = value.as_bytes();
    let start = bytes
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t'))
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|byte| !matches!(byte, b' ' | b'\t'))
        .map_or(start, |index| index + 1);
    HeaderValue::from_bytes(&bytes[start..end]).expect("trimmed header value remains valid")
}

/// Converts a validated header value into the string form exposed by [`Headers`].
pub(super) fn header_value_string(value: &HeaderValue) -> String {
    String::from_utf8_lossy(value.as_bytes()).into_owned()
}

/// Identifies response headers that must retain separate entries.
pub(super) fn is_non_combinable(name: &HeaderName) -> bool {
    matches!(name.as_str(), "set-cookie" | "set-cookie2")
}
