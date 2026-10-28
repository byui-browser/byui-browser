//! Fetch form-data values plus URL-encoded and multipart conversion.

use http::header::HeaderValue;

use super::{Blob, RequestError};

/// An ordered collection of form fields and blob-like file parts.
///
/// This initial engine representation supports URL-encoded fields and the
/// multipart field/file subset used by [`super::Body::form_data`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FormData {
    pub(super) entries: Vec<(String, FormDataEntry)>,
}

/// A value stored in [`FormData`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FormDataEntry {
    /// A textual form field.
    Text(String),
    /// A blob-like file part with an optional file name.
    File {
        /// Bytes and media type for this file part.
        blob: Blob,
        /// File name supplied by the caller, when any.
        filename: Option<String>,
    },
}

impl FormData {
    /// Creates an empty form-data collection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a textual field, preserving insertion order and duplicates.
    pub fn append(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.entries
            .push((name.into(), FormDataEntry::Text(value.into())));
    }

    /// Appends a blob-like file field, preserving insertion order and duplicates.
    pub fn append_file(&mut self, name: impl Into<String>, blob: Blob, filename: Option<String>) {
        self.entries
            .push((name.into(), FormDataEntry::File { blob, filename }));
    }

    /// Iterates over field names and values in insertion order.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &FormDataEntry)> {
        self.entries
            .iter()
            .map(|(name, value)| (name.as_str(), value))
    }
}

/// Serializes form data and returns its bytes and inferred media type.
pub(super) fn serialize(form: FormData) -> (Vec<u8>, HeaderValue) {
    if form
        .entries
        .iter()
        .all(|(_, value)| matches!(value, FormDataEntry::Text(_)))
    {
        let mut serializer = url::form_urlencoded::Serializer::new(String::new());
        for (name, value) in form.entries {
            if let FormDataEntry::Text(value) = value {
                serializer.append_pair(&name, &value);
            }
        }
        return (
            serializer.finish().into_bytes(),
            HeaderValue::from_static("application/x-www-form-urlencoded;charset=UTF-8"),
        );
    }

    const BOUNDARY: &str = "----net-fetch-form-data";
    let mut bytes = Vec::new();
    for (name, value) in form.entries {
        bytes.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        let name = escape_quoted_parameter(&name);
        match value {
            FormDataEntry::Text(value) => bytes.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n")
                    .as_bytes(),
            ),
            FormDataEntry::File { blob, filename } => {
                let filename = filename.unwrap_or_else(|| "blob".into());
                let quoted_filename = escape_quoted_parameter(&filename);
                bytes.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{name}\"; filename=\"{quoted_filename}\""
                    )
                    .as_bytes(),
                );
                if !filename.is_ascii() {
                    bytes.extend_from_slice(b"; filename*=UTF-8''");
                    bytes.extend_from_slice(encode_rfc5987(&filename).as_bytes());
                }
                bytes.extend_from_slice(b"\r\n");
                if let Some(media_type) = blob.media_type() {
                    bytes.extend_from_slice(b"Content-Type: ");
                    bytes.extend_from_slice(media_type.as_bytes());
                    bytes.extend_from_slice(b"\r\n");
                }
                bytes.extend_from_slice(b"\r\n");
                bytes.extend_from_slice(blob.bytes());
                bytes.extend_from_slice(b"\r\n");
            }
        }
    }
    bytes.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    (
        bytes,
        HeaderValue::from_static("multipart/form-data; boundary=----net-fetch-form-data"),
    )
}

/// Parses URL-encoded or supported multipart form bytes.
pub(super) fn parse(content_type: &str, bytes: &[u8]) -> Result<FormData, RequestError> {
    if content_type.split(';').next().is_some_and(|kind| {
        kind.trim()
            .eq_ignore_ascii_case("application/x-www-form-urlencoded")
    }) {
        let mut form = FormData::new();
        for (name, value) in url::form_urlencoded::parse(bytes) {
            form.append(name, value);
        }
        return Ok(form);
    }
    let boundary = multipart_boundary(content_type).ok_or_else(|| {
        RequestError::BodyFormData("expected URL-encoded or multipart Content-Type".into())
    })?;
    parse_multipart(bytes, boundary)
}

/// Extracts the boundary parameter from a multipart media type.
fn multipart_boundary(content_type: &str) -> Option<&str> {
    content_type.split(';').skip(1).find_map(|parameter| {
        let (name, value) = parameter.trim().split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case("boundary")
            .then(|| value.trim().trim_matches('"'))
    })
}

/// Parses multipart parts into text entries or blob-backed file entries.
fn parse_multipart(bytes: &[u8], boundary: &str) -> Result<FormData, RequestError> {
    if boundary.is_empty() || boundary.bytes().any(|byte| byte <= b' ' || byte >= 0x7f) {
        return Err(RequestError::BodyFormData(
            "multipart boundary is invalid".into(),
        ));
    }
    let delimiter = format!("--{boundary}").into_bytes();
    let mut position = find_bytes(bytes, &delimiter, 0)
        .ok_or_else(|| RequestError::BodyFormData("multipart boundary is missing".into()))?;
    let mut form = FormData::new();
    loop {
        position += delimiter.len();
        if bytes.get(position..position + 2) == Some(b"--") {
            break;
        }
        if bytes.get(position..position + 2) != Some(b"\r\n") {
            return Err(RequestError::BodyFormData(
                "multipart boundary has an invalid suffix".into(),
            ));
        }
        position += 2;
        let header_end = find_bytes(bytes, b"\r\n\r\n", position).ok_or_else(|| {
            RequestError::BodyFormData("multipart part is missing its header separator".into())
        })?;
        let headers = std::str::from_utf8(&bytes[position..header_end])
            .map_err(|_| RequestError::BodyFormData("multipart headers are not UTF-8".into()))?;
        let value_start = header_end + 4;
        let next_boundary =
            find_multipart_boundary(bytes, &delimiter, value_start).ok_or_else(|| {
                RequestError::BodyFormData("multipart closing boundary is missing".into())
            })?;
        let value = &bytes[value_start..next_boundary];
        let disposition = headers
            .lines()
            .find(|line| {
                line.to_ascii_lowercase()
                    .starts_with("content-disposition:")
            })
            .ok_or_else(|| RequestError::BodyFormData("multipart part lacks disposition".into()))?;
        let name = disposition_parameter(disposition, "name")
            .ok_or_else(|| RequestError::BodyFormData("multipart part lacks name".into()))?;
        let filename = disposition_parameter(disposition, "filename*")
            .and_then(|value| decode_rfc5987(&value))
            .or_else(|| disposition_parameter(disposition, "filename"));
        if let Some(filename) = filename {
            let media_type = headers.lines().find_map(|line| {
                line.split_once(':').and_then(|(name, value)| {
                    name.trim()
                        .eq_ignore_ascii_case("content-type")
                        .then_some(value.trim())
                        .and_then(|value| HeaderValue::try_from(value).ok())
                })
            });
            form.append_file(name, Blob::new(value.to_vec(), media_type), Some(filename));
        } else {
            form.append(name, String::from_utf8_lossy(value));
        }
        position = next_boundary + 2;
    }
    Ok(form)
}

/// Reads a named parameter from a multipart content-disposition header.
fn disposition_parameter(disposition: &str, requested: &str) -> Option<String> {
    let bytes = disposition.as_bytes();
    let mut position = disposition.find(';')? + 1;
    while position < bytes.len() {
        while bytes
            .get(position)
            .is_some_and(|byte| byte.is_ascii_whitespace() || *byte == b';')
        {
            position += 1;
        }
        let name_start = position;
        while bytes
            .get(position)
            .is_some_and(|byte| *byte != b'=' && *byte != b';')
        {
            position += 1;
        }
        let name = disposition[name_start..position].trim();
        if bytes.get(position) != Some(&b'=') {
            position += usize::from(position < bytes.len());
            continue;
        }
        position += 1;
        while bytes.get(position).is_some_and(u8::is_ascii_whitespace) {
            position += 1;
        }
        let value = if bytes.get(position) == Some(&b'"') {
            position += 1;
            let mut value_bytes = Vec::new();
            while let Some(&byte) = bytes.get(position) {
                position += 1;
                match byte {
                    b'"' => break,
                    b'\\' => {
                        if let Some(&escaped) = bytes.get(position) {
                            value_bytes.push(escaped);
                            position += 1;
                        }
                    }
                    _ => value_bytes.push(byte),
                }
            }
            String::from_utf8(value_bytes).ok()?
        } else {
            let value_start = position;
            while bytes.get(position).is_some_and(|byte| *byte != b';') {
                position += 1;
            }
            disposition[value_start..position].trim().to_owned()
        };
        if name.eq_ignore_ascii_case(requested) {
            return Some(value);
        }
    }
    None
}

/// Finds a byte substring beginning at `start`.
fn find_bytes(haystack: &[u8], needle: &[u8], start: usize) -> Option<usize> {
    if needle.is_empty() || start > haystack.len() {
        return None;
    }
    haystack[start..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|position| start + position)
}

/// Finds the CRLF that begins the next syntactically valid multipart delimiter.
fn find_multipart_boundary(bytes: &[u8], delimiter: &[u8], start: usize) -> Option<usize> {
    let mut marker = Vec::with_capacity(delimiter.len() + 2);
    marker.extend_from_slice(b"\r\n");
    marker.extend_from_slice(delimiter);
    let mut search = start;
    while let Some(position) = find_bytes(bytes, &marker, search) {
        let suffix = position + marker.len();
        if matches!(bytes.get(suffix..suffix + 2), Some(b"\r\n") | Some(b"--")) {
            return Some(position);
        }
        search = position + 1;
    }
    None
}

/// Escapes a quoted multipart parameter without admitting header delimiters.
fn escape_quoted_parameter(value: &str) -> String {
    value
        .replace('\r', "%0D")
        .replace('\n', "%0A")
        .replace('"', "%22")
}

/// Encodes a UTF-8 RFC 5987 extended parameter value.
fn encode_rfc5987(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[(byte >> 4) as usize]));
            encoded.push(char::from(HEX[(byte & 0x0f) as usize]));
        }
    }
    encoded
}

/// Decodes an RFC 5987 UTF-8 extended parameter value.
fn decode_rfc5987(value: &str) -> Option<String> {
    let (charset, remainder) = value.split_once('\'')?;
    let (_, encoded) = remainder.split_once('\'')?;
    if !charset.eq_ignore_ascii_case("utf-8") {
        return None;
    }
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut position = 0;
    while position < bytes.len() {
        if bytes[position] == b'%' {
            let high = hex_value(*bytes.get(position + 1)?)?;
            let low = hex_value(*bytes.get(position + 2)?)?;
            decoded.push((high << 4) | low);
            position += 3;
        } else {
            decoded.push(bytes[position]);
            position += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
