//! Body consumption helpers for bytes, text, structured data, and form data.

use encoding_rs::Encoding;
use futures_util::StreamExt;
use serde::de::DeserializeOwned;

use super::{Blob, Body, FormData, RequestError, form_data, metadata};

impl Body {
    /// Consumes this body into owned bytes.
    pub async fn bytes(mut self) -> Result<Vec<u8>, RequestError> {
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| RequestError::BodyAlreadyUsed)?;
            if state.used || state.locked {
                log::debug!("body consume rejected: already used or locked");
                return Err(RequestError::BodyAlreadyUsed);
            }
            state.locked = true;
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = self.next().await {
            bytes.extend_from_slice(&chunk?);
        }
        log::debug!("body consumed: {} bytes", bytes.len());
        Ok(bytes)
    }

    /// Consumes this body into an ArrayBuffer-compatible byte vector.
    pub async fn array_buffer(self) -> Result<Vec<u8>, RequestError> {
        self.bytes().await
    }

    /// Consumes this body as text using its declared charset or UTF-8.
    ///
    /// Byte-order marks are handled by the selected encoding and malformed
    /// sequences are replaced, matching Fetch's text-decoding behavior.
    pub async fn text(self) -> Result<String, RequestError> {
        let content_type = self.content_type();
        let bytes = self.bytes().await?;
        let encoding = content_type
            .as_ref()
            .and_then(metadata::charset_from_content_type)
            .and_then(|charset| Encoding::for_label(charset.as_bytes()))
            .unwrap_or(encoding_rs::UTF_8);
        let (text, _, had_errors) = encoding.decode(&bytes);
        if had_errors {
            log::warn!(
                "body text had malformed {} sequences; replacement characters inserted",
                encoding.name()
            );
        }
        Ok(text.into_owned())
    }

    /// Consumes this body as a blob-like byte payload.
    pub async fn blob(self) -> Result<Blob, RequestError> {
        let media_type = self.content_type();
        Ok(Blob::new(self.bytes().await?, media_type))
    }

    /// Consumes this body and deserializes JSON into the requested Rust type.
    pub async fn json<T: DeserializeOwned>(self) -> Result<T, RequestError> {
        let text = self.text().await?;
        serde_json::from_str(&text).map_err(|error| {
            log::debug!("body json parse failed: {error}");
            RequestError::BodyJson(error.to_string())
        })
    }

    /// Consumes this body as URL-encoded or multipart form data.
    pub async fn form_data(self) -> Result<FormData, RequestError> {
        let content_type = self
            .content_type()
            .ok_or_else(|| RequestError::BodyFormData("missing Content-Type".into()))?;
        let content_type = content_type
            .to_str()
            .map_err(|_| RequestError::BodyFormData("invalid Content-Type".into()))?;
        let bytes = self.bytes().await?;
        form_data::parse(content_type, &bytes)
    }
}
