//! Configuration for the reqwest client and its connection pool.

use std::time::Duration;

use http::HeaderValue;

/// Configuration used when constructing a [`RequestController`](crate::RequestController).
#[derive(Clone, Debug)]
pub struct Config {
    /// How long pooled idle connections are retained. `None` disables the timeout.
    pub pool_idle_timeout: Option<Duration>,
    /// Maximum number of idle connections retained for each host.
    pub pool_max_idle_per_host: usize,
    /// Optional `User-Agent` header applied to outgoing requests.
    pub user_agent: Option<HeaderValue>,
    /// Maximum number of requests allowed to execute concurrently.
    pub max_in_flight: usize,
    /// Maximum known request-body size, in bytes, for requests marked `keepalive`.
    pub max_keepalive_body_size: u64,
}

impl Default for Config {
    /// Returns conservative browser defaults for pooling and identity.
    fn default() -> Self {
        Self {
            pool_idle_timeout: Some(Duration::from_secs(90)),
            pool_max_idle_per_host: 8,
            user_agent: Some(HeaderValue::from_static("byui-browser/0.1")),
            max_in_flight: 32,
            max_keepalive_body_size: 65_536,
        }
    }
}
