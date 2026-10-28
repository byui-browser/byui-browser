use url::Url;

use crate::api::error::RequestError;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_and_non_default_ports_canonically() {
        let https = Origin::parse("https://EXAMPLE.test").unwrap();
        let https_default = Origin::parse("https://example.test:443").unwrap();
        let https_non_default = Origin::parse("https://example.test:8443").unwrap();
        let http = Origin::parse("http://example.test").unwrap();

        assert_eq!(https.as_str(), "https://example.test");
        assert!(https.is_same_origin(&https_default));
        assert_eq!(https_non_default.as_str(), "https://example.test:8443");
        assert!(!https.is_same_origin(&https_non_default));
        assert!(!https.is_same_origin(&http));
    }

    #[test]
    fn parse_rejects_non_origin_serializations() {
        for value in [
            "https://example.test/path",
            "https://example.test/?query=one",
            "https://example.test/#fragment",
            "https://user@example.test/",
            "ftp://example.test/",
            "not an origin",
        ] {
            assert!(matches!(
                Origin::parse(value),
                Err(RequestError::InvalidOrigin(actual)) if actual == value
            ));
        }
    }

    #[test]
    fn opaque_origins_do_not_match_any_origin_including_themselves() {
        let first = Origin::opaque();
        let second = Origin::opaque();
        let tuple = Origin::parse("https://example.test").unwrap();

        assert!(first.is_opaque());
        assert!(!first.is_same_origin(&first));
        assert!(!first.is_same_origin(&second));
        assert!(!first.is_same_origin(&tuple));
    }
}
