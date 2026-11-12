//! Shared media-type validation for Fetch metadata.

use http::HeaderValue;

/// Returns whether a header value has the supported `type/subtype; name=value` shape.
pub(crate) fn is_valid_media_type(value: &HeaderValue) -> bool {
    let Ok(value) = value.to_str() else {
        return false;
    };
    let mut parts = value.split(';');
    let Some(essence) = parts.next() else {
        return false;
    };
    let Some((kind, subtype)) = essence.trim().split_once('/') else {
        return false;
    };
    if kind.is_empty()
        || subtype.is_empty()
        || kind.bytes().any(|byte| byte.is_ascii_whitespace())
        || subtype.bytes().any(|byte| byte.is_ascii_whitespace())
    {
        return false;
    }
    parts.all(|parameter| {
        parameter
            .split_once('=')
            .is_some_and(|(name, value)| !name.trim().is_empty() && !value.trim().is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_shared_media_type_rules() {
        for value in [
            "text/plain",
            "text/plain;charset=utf-8",
            "application/problem+json; profile=example",
        ] {
            assert!(is_valid_media_type(&value.parse().unwrap()), "{value}");
        }
        for value in [
            "textplain",
            "text /plain",
            "text/ plain",
            "text/plain;charset",
            "text/plain;=utf-8",
        ] {
            assert!(!is_valid_media_type(&value.parse().unwrap()), "{value}");
        }
    }
}
