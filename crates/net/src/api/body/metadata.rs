//! Content-type parsing helpers for body consumption.

use http::header::HeaderValue;

/// Returns the declared charset parameter, if present.
pub(super) fn charset_from_content_type(value: &HeaderValue) -> Option<String> {
    let value = value.to_str().ok()?;
    value.split(';').skip(1).find_map(|parameter| {
        let (name, value) = parameter.trim().split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case("charset")
            .then(|| value.trim().trim_matches('"').to_owned())
    })
}
