use url::Url;

use crate::error::RequestError;

/// Structured origin state for a client environment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Origin {
    scheme: Option<String>,
    host: Option<String>,
    port: Option<u16>,
}

impl Origin {
    /// Creates the tuple origin for an HTTP(S) URL.
    pub fn from_url(url: &Url) -> Result<Self, RequestError> {
        if !matches!(url.scheme(), "http" | "https") {
            return Err(RequestError::UnsupportedScheme(url.scheme().to_owned()));
        }
        let host = url
            .host_str()
            .ok_or_else(|| RequestError::InvalidUrl(url.as_str().to_owned()))?;
        Ok(Self {
            scheme: Some(url.scheme().to_ascii_lowercase()),
            host: Some(host.to_ascii_lowercase()),
            port: url.port_or_known_default(),
        })
    }

    /// Creates an opaque origin, which is unequal to every tuple origin.
    pub fn opaque() -> Self {
        Self {
            scheme: None,
            host: None,
            port: None,
        }
    }

    /// Parses a serialized HTTP(S) origin and rejects paths, credentials, and queries.
    pub fn parse(serialized: &str) -> Result<Self, RequestError> {
        let url = Url::parse(serialized)
            .map_err(|_| RequestError::InvalidOrigin(serialized.to_owned()))?;
        if url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(RequestError::InvalidOrigin(serialized.to_owned()));
        }
        Self::from_url(&url).map_err(|_| RequestError::InvalidOrigin(serialized.to_owned()))
    }

    /// Returns this origin's canonical serialization, or `"null"` for opaque origins.
    pub fn as_str(&self) -> String {
        let (Some(scheme), Some(host), Some(port)) = (&self.scheme, &self.host, self.port) else {
            return "null".to_owned();
        };
        let default_port = match scheme.as_str() {
            "http" => 80,
            "https" => 443,
            _ => port,
        };
        if port == default_port {
            format!("{scheme}://{host}")
        } else {
            format!("{scheme}://{host}:{port}")
        }
    }

    /// Returns whether this is an opaque origin.
    pub fn is_opaque(&self) -> bool {
        self.scheme.is_none()
    }

    /// Tests tuple-origin equality; opaque origins never match, including themselves.
    pub fn is_same_origin(&self, other: &Self) -> bool {
        !self.is_opaque() && !other.is_opaque() && self == other
    }
}

/// Top-level origin used to partition browser network state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPartitionKey {
    /// Origin of the top-level site containing the request's client.
    pub top_level_origin: Origin,
}

impl NetworkPartitionKey {
    /// Creates a partition key from the top-level site's structured origin.
    pub fn new(top_level_origin: Origin) -> Self {
        Self { top_level_origin }
    }
}
