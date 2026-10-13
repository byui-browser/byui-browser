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
        match value {
            FormDataEntry::Text(value) => bytes.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n")
                    .as_bytes(),
            ),
            FormDataEntry::File { blob, filename } => {
                let filename = filename.unwrap_or_else(|| "blob".into());
                bytes.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\n"
                    )
                    .as_bytes(),
                );
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

fn multipart_boundary(content_type: &str) -> Option<&str> {
    content_type.split(';').skip(1).find_map(|parameter| {
        let (name, value) = parameter.trim().split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case("boundary")
            .then(|| value.trim().trim_matches('"'))
    })
}

fn parse_multipart(bytes: &[u8], boundary: &str) -> Result<FormData, RequestError> {
    let boundary = format!("--{boundary}");
    let body = std::str::from_utf8(bytes)
        .map_err(|_| RequestError::BodyFormData("multipart body is not UTF-8".into()))?;
    let mut form = FormData::new();
    for part in body.split(&boundary).skip(1) {
        let part = part.trim_start_matches("\r\n");
        if part.starts_with("--") {
            break;
        }
        let (headers, value) = part.split_once("\r\n\r\n").ok_or_else(|| {
            RequestError::BodyFormData("multipart part is missing its header separator".into())
        })?;
        let value = value.strip_suffix("\r\n").unwrap_or(value);
        let disposition = headers
            .lines()
            .find(|line| {
                line.to_ascii_lowercase()
                    .starts_with("content-disposition:")
            })
            .ok_or_else(|| RequestError::BodyFormData("multipart part lacks disposition".into()))?;
        let name = disposition_parameter(disposition, "name")
            .ok_or_else(|| RequestError::BodyFormData("multipart part lacks name".into()))?;
        if let Some(filename) = disposition_parameter(disposition, "filename") {
            let media_type = headers.lines().find_map(|line| {
                line.split_once(':').and_then(|(name, value)| {
                    name.trim()
                        .eq_ignore_ascii_case("content-type")
                        .then_some(value.trim())
                        .and_then(|value| HeaderValue::try_from(value).ok())
                })
            });
            form.append_file(
                name,
                Blob::new(value.as_bytes().to_vec(), media_type),
                Some(filename),
            );
        } else {
            form.append(name, value);
        }
    }
    Ok(form)
}

fn disposition_parameter(disposition: &str, requested: &str) -> Option<String> {
    disposition.split(';').skip(1).find_map(|parameter| {
        let (name, value) = parameter.trim().split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case(requested)
            .then(|| value.trim().trim_matches('"').to_owned())
    })
}
