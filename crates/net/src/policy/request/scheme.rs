use crate::api::error::RequestError;

/// URL scheme selected by the Fetch controller after URL parsing.
///
/// Non-HTTP variants are classified here so the controller can select a
/// scheme-specific implementation without coupling URL validation to HTTP.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FetchScheme {
    /// HTTP network fetch.
    Http,
    /// HTTPS network fetch.
    Https,
    /// Inline data URL fetch.
    Data,
    /// Browser-owned object URL fetch.
    Blob,
    /// Provider-authorized local file fetch.
    File,
    /// Browser-owned local page fetch.
    Byui,
    /// Supported browser-defined `about:` fetch.
    About,
}

impl FetchScheme {
    /// Returns the canonical scheme name used by provider diagnostics.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
            Self::Data => "data",
            Self::Blob => "blob",
            Self::File => "file",
            Self::Byui => "byui",
            Self::About => "about",
        }
    }

    /// Classifies a parsed URL or returns the typed unsupported-scheme error.
    pub(crate) fn from_url(url: &reqwest::Url) -> Result<Self, RequestError> {
        match url.scheme() {
            "http" => Ok(Self::Http),
            "https" => Ok(Self::Https),
            "data" => Ok(Self::Data),
            "blob" => Ok(Self::Blob),
            "file" => Ok(Self::File),
            "byui" => Ok(Self::Byui),
            "about" => Ok(Self::About),
            scheme => {
                log::warn!("unsupported url scheme: {scheme}");
                Err(RequestError::UnsupportedScheme(scheme.to_owned()))
            }
        }
    }

    /// Returns whether this scheme uses the existing HTTP network path.
    pub(crate) fn is_http(self) -> bool {
        matches!(self, Self::Http | Self::Https)
    }
}
