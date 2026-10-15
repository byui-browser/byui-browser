//! Engine-owned Fetch scheme handlers.

use base64::{Engine as _, engine::general_purpose::STANDARD};
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
};

impl RequestController {
    /// Fetches an inline `data:` URL without invoking transport or providers.
    pub(crate) async fn fetch_data(
        &self,
        request: Request,
        url: Url,
    ) -> Result<Response, RequestError> {
        let (media_type, bytes) = parse_data_url(&url)?;
        let resource = SchemeResource {
            url,
            status: 200,
            status_text: Some("OK".into()),
            headers: vec![(HeaderName::from_static("content-type"), media_type)],
            media_type: None,
            origin: Origin::opaque(),
            body: if request.method() == Method::HEAD {
                SchemeBody::Null
            } else {
                SchemeBody::Bytes(bytes)
            },
        };
        self.finalize_scheme_resource(&request, resource, crate::policy::FetchScheme::Data)
    }
}

/// Parses a data URL into validated media-type metadata and decoded bytes.
fn parse_data_url(url: &Url) -> Result<(HeaderValue, Vec<u8>), RequestError> {
    let value = url
        .as_str()
        .strip_prefix("data:")
        .ok_or_else(|| RequestError::InvalidUrl(url.to_string()))?;
    let (metadata, payload) = value
        .split_once(',')
        .ok_or_else(|| invalid_data("missing comma separator"))?;

    let mut media_parts = Vec::new();
    let mut is_base64 = false;
    for part in metadata.split(';') {
        if part.eq_ignore_ascii_case("base64") {
            if is_base64 {
                return Err(invalid_data("duplicate base64 marker"));
            }
            is_base64 = true;
        } else if !part.is_empty() {
            media_parts.push(part);
        }
    }

    let media_type = if media_parts.is_empty() {
        HeaderValue::from_static("text/plain;charset=US-ASCII")
    } else {
        let media_type = percent_decode_text(&media_parts.join(";"))?;
        validate_media_type(&media_type)?;
        HeaderValue::from_str(&media_type)
            .map_err(|_| invalid_data("invalid Content-Type header value"))?
    };

    let decoded_payload = percent_decode(payload.as_bytes())?;
    let bytes = if is_base64 {
        let encoded = std::str::from_utf8(&decoded_payload)
            .map_err(|_| invalid_data("base64 payload is not ASCII"))?;
        STANDARD
            .decode(encoded)
            .map_err(|_| invalid_data("invalid base64 payload"))?
    } else {
        decoded_payload
    };
    Ok((media_type, bytes))
}

/// Validates the media type embedded in a data URL.
fn validate_media_type(value: &str) -> Result<(), RequestError> {
    let mut parts = value.split(';');
    let essence = parts.next().unwrap_or_default().trim();
    let Some((kind, subtype)) = essence.split_once('/') else {
        return Err(invalid_data("media type must contain a type and subtype"));
    };
    if kind.is_empty() || subtype.is_empty() || kind.contains(char::is_whitespace) {
        return Err(invalid_data("invalid media type"));
    }
    for parameter in parts {
        let Some((name, value)) = parameter.split_once('=') else {
            return Err(invalid_data("media type parameter is missing '='"));
        };
        if name.trim().is_empty() || value.trim().is_empty() {
            return Err(invalid_data("invalid media type parameter"));
        }
    }
    Ok(())
}

/// Percent-decodes data URL metadata and requires valid UTF-8 text.
fn percent_decode_text(value: &str) -> Result<String, RequestError> {
    let bytes = percent_decode(value.as_bytes())?;
    String::from_utf8(bytes).map_err(|_| invalid_data("media type is not UTF-8"))
}

/// Decodes percent escapes while preserving all other payload bytes.
fn percent_decode(value: &[u8]) -> Result<Vec<u8>, RequestError> {
    let mut output = Vec::with_capacity(value.len());
    let mut index = 0;
    while index < value.len() {
        if value[index] != b'%' {
            output.push(value[index]);
            index += 1;
            continue;
        }
        if index + 2 >= value.len() {
            return Err(invalid_data("incomplete percent escape"));
        }
        let high = hex(value[index + 1]).ok_or_else(|| invalid_data("invalid percent escape"))?;
        let low = hex(value[index + 2]).ok_or_else(|| invalid_data("invalid percent escape"))?;
        output.push((high << 4) | low);
        index += 3;
    }
    Ok(output)
}

/// Converts one ASCII hexadecimal digit to its numeric value.
fn hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

/// Creates the stable error category used for malformed data URLs.
fn invalid_data(message: &'static str) -> RequestError {
    RequestError::InvalidResponse(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// Verifies plain, percent-encoded, and base64 data payloads decode.
    fn parses_plain_percent_encoded_and_base64_data() {
        assert_eq!(
            parse_data_url(&Url::parse("data:,hello").unwrap())
                .unwrap()
                .1,
            b"hello"
        );
        assert_eq!(
            parse_data_url(&Url::parse("data:text/plain,hello%20world").unwrap())
                .unwrap()
                .1,
            b"hello world"
        );
        assert_eq!(
            parse_data_url(&Url::parse("data:text/plain;base64,SGVsbG8=").unwrap())
                .unwrap()
                .1,
            b"Hello"
        );
    }

    #[test]
    /// Verifies malformed payloads and media types fail with typed errors.
    fn rejects_malformed_data_payloads_and_media_types() {
        for value in [
            "data:,bad%",
            "data:text/plain;base64,not-base64!",
            "data:textplain,x",
        ] {
            assert!(parse_data_url(&Url::parse(value).unwrap()).is_err());
        }
    }
}
