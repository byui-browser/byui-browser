//! Sandboxing, process isolation helpers, CSP, permissions, and cookie policy.
//!
//! **Owning team**: Security & Storage Team
//!
//! Renderer processes are heavily sandboxed. Any new capability (file access,
//! device APIs, etc.) must be reviewed by this team.

#![forbid(unsafe_code)]

use std::fmt;

/// A web origin: scheme, host, port. Two documents may touch each other's
/// state only when their origins are equal.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Reshape the
// types, names, and module layout however your crate's public API needs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Origin {
    pub scheme: String,
    pub host: String,
    pub port: u16,
}

impl Origin {
    /// Creates an origin, using the scheme's default port when omitted.
    pub fn new(scheme: impl Into<String>, host: impl Into<String>, port: Option<u16>) -> Self {
        let scheme = scheme.into().to_ascii_lowercase();
        let host = host.into().to_ascii_lowercase();
        let port = port.unwrap_or_else(|| match scheme.as_str() {
            "http" => 80,
            "https" => 443,
            _ => 0,
        });

        Self {
            scheme,
            host,
            port,
        }
    }

    /// Creates an origin with an explicitly specified port.
    pub fn with_port(
        scheme: impl Into<String>,
        host: impl Into<String>,
        port: u16,
    ) -> Self {
        Self::new(scheme, host, Some(port))
    }

    /// Creates a clearly identifiable origin for tests and early integration work.
    // TODO(integration): Replace this fixture with the origin supplied by the
    // Networking/HTML URL implementation once those teams expose it.
    pub fn placeholder() -> Self {
        Self::new("https", "placeholder.invalid", None)
    }

    /// Extracts the origin from an absolute URL.
    ///
    /// Currently only rejects the empty string.
    // NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
    // TODO(integration): Replace this stub with the shared URL parser once the
    // Networking team provides scheme, host, and port extraction.
    pub fn parse(url: &str) -> Result<Self, SecurityError> {
        if url.is_empty() {
            return Err(SecurityError::InvalidOrigin(String::new()));
        }
        todo!("TODO(security): extract origin from: {url:?}")
    }
}

/// The key used to associate browser storage with an origin.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StorageKey {
    origin: Origin,
}

impl StorageKey {
    /// Creates a storage key for `origin`.
    pub fn new(origin: Origin) -> Self {
        Self { origin }
    }

    /// Returns the origin associated with this key.
    pub fn origin(&self) -> &Origin {
        &self.origin
    }
}

/// Same-origin policy check.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
pub fn same_origin(a: &Origin, b: &Origin) -> bool {
    a == b
}

/// A cookie as stored in the jar.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub secure: bool,
}

/// Cookie storage with per-origin visibility rules. A Canvas session is a
/// cookie, so this is on the critical path to the course goal.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CookieJar {
    cookies: Vec<Cookie>,
}

impl CookieJar {
    /// An empty jar.
    pub fn new() -> Self {
        Self::default()
    }

    /// Stores a cookie received from `origin`, applying domain/secure rules.
    // NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
    pub fn store(&mut self, origin: &Origin, cookie: Cookie) {
        todo!("TODO(security): store {} for {}", cookie.name, origin.host)
    }

    /// Cookies that may be sent to `origin`.
    ///
    /// Currently only handles the empty jar.
    // NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
    pub fn cookies_for(&self, origin: &Origin) -> Vec<&Cookie> {
        if self.cookies.is_empty() {
            return Vec::new();
        }
        todo!(
            "TODO(security): filter {} cookies for {}",
            self.cookies.len(),
            origin.host
        )
    }
}

/// Security policy failure.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecurityError {
    InvalidOrigin(String),
}

impl fmt::Display for SecurityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOrigin(u) => write!(f, "invalid origin: {u:?}"),
        }
    }
}

impl std::error::Error for SecurityError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(scheme: &str, host: &str, port: u16) -> Origin {
        // Once we get a URL parser, we can use it here to test the parse() function. For now, just construct the origin directly.
        Origin::with_port(
            scheme,
            host,
            port)
    }

    #[test]
    fn empty_url_has_no_origin() {
        assert_eq!(
            Origin::parse(""),
            Err(SecurityError::InvalidOrigin(String::new()))
        );
    }

    #[test]
    fn identical_origins_are_same_origin() {
        let a = origin("https", "byui.edu", 443);
        assert!(same_origin(&a, &a.clone()));
    }

    #[test]
    fn differing_port_or_scheme_is_cross_origin() {
        let a = origin("https", "byui.edu", 443);
        assert!(!same_origin(&a, &origin("https", "byui.edu", 8443)));
        assert!(!same_origin(&a, &origin("http", "byui.edu", 443)));
    }

    #[test]
    fn differing_hosts_are_cross_origin() {
        assert_ne!(
            origin("https", "byui.edu", 443),
            origin("https", "example.com", 443)
        );
    }

    #[test]
    fn default_ports_are_scheme_specific() {
        assert_eq!(Origin::new("http", "example.com", None).port, 80);
        assert_eq!(Origin::new("https", "example.com", None).port, 443);
        assert_ne!(
            Origin::new("http", "example.com", None),
            Origin::new("https", "example.com", None)
        );
    }

    #[test]
    fn scheme_and_host_are_normalized() {
        assert_eq!(
            Origin::new("HTTPS", "EXAMPLE.COM", None),
            Origin::new("https", "example.com", Some(443))
        );
    }

    #[test]
    fn placeholder_origin_can_create_storage_key() {
        let origin = Origin::placeholder();
        let key = StorageKey::new(origin.clone());

        assert_eq!(key.origin(), &origin);
    }

    #[test]
    fn storage_key_identifies_its_origin() {
        let origin = Origin::new("https", "byui.edu", None);
        let key = StorageKey::new(origin.clone());

        assert_eq!(key.origin(), &origin);
        assert_eq!(key, StorageKey::new(origin));
    }

    #[test]
    fn empty_jar_sends_nothing() {
        let jar = CookieJar::new();
        assert!(
            jar.cookies_for(&origin("https", "byui.edu", 443))
                .is_empty()
        );
    }
}
