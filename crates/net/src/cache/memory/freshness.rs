//! HTTP response freshness policy and time sources.

use std::{
    fmt,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use reqwest::header::{AGE, CACHE_CONTROL, DATE, EXPIRES, HeaderMap};

/// Clock reading used for cache freshness and deterministic tests.
#[derive(Clone, Copy, Debug)]
pub(super) struct CacheTime {
    pub(super) wall: Duration,
    pub(super) elapsed: Duration,
}

/// Source of wall-clock and monotonic time for cache freshness calculations.
pub(super) trait CacheClock: fmt::Debug + Send + Sync {
    fn now(&self) -> CacheTime;
}

/// Production clock with a monotonic origin shared by one cache instance.
#[derive(Debug)]
pub(super) struct SystemCacheClock {
    pub(super) started: Instant,
}

impl CacheClock for SystemCacheClock {
    fn now(&self) -> CacheTime {
        CacheTime {
            wall: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO),
            elapsed: self.started.elapsed(),
        }
    }
}

/// Parsed response directives that affect the supported freshness subset.
#[derive(Clone, Copy, Debug, Default)]
struct CacheControl {
    max_age: Option<Duration>,
    no_cache: bool,
    no_store: bool,
    must_revalidate: bool,
}

/// Stored age and freshness state for one cached response.
#[derive(Clone, Copy, Debug)]
pub(super) struct FreshnessMetadata {
    pub(super) response_time: CacheTime,
    pub(super) date: Option<Duration>,
    pub(super) age: Duration,
    pub(super) lifetime: Duration,
    pub(super) requires_revalidation: bool,
    pub(super) must_revalidate: bool,
}

impl FreshnessMetadata {
    /// Builds cache metadata for the supported explicit-freshness subset.
    ///
    /// Responses without `max-age` or a valid `Expires` value are not stored;
    /// heuristic freshness is intentionally deferred.
    pub(super) fn from_headers(headers: &HeaderMap, response_time: CacheTime) -> Option<Self> {
        let directives = cache_control(headers);
        if directives.no_store {
            return None;
        }
        let date = parse_http_date(headers.get(DATE));
        let expires = parse_http_date(headers.get(EXPIRES));
        let lifetime = if directives.no_cache {
            Duration::ZERO
        } else if let Some(max_age) = directives.max_age {
            max_age
        } else {
            expires?.saturating_sub(date.unwrap_or(response_time.wall))
        };
        Some(Self {
            response_time,
            date,
            age: parse_age(headers),
            lifetime,
            requires_revalidation: directives.no_cache,
            must_revalidate: directives.must_revalidate,
        })
    }

    pub(super) fn is_fresh(&self, now: CacheTime) -> bool {
        !self.requires_revalidation && self.current_age(now) < self.lifetime
    }

    pub(super) fn may_serve_stale(&self) -> bool {
        !self.requires_revalidation && !self.must_revalidate
    }

    fn current_age(&self, now: CacheTime) -> Duration {
        let apparent_age = self
            .date
            .map(|date| self.response_time.wall.saturating_sub(date))
            .unwrap_or(Duration::ZERO);
        let corrected_received_age = apparent_age.max(self.age);
        corrected_received_age
            .saturating_add(now.elapsed.saturating_sub(self.response_time.elapsed))
    }
}

/// Parses the supported `Cache-Control` directives without retaining raw values.
fn cache_control(headers: &HeaderMap) -> CacheControl {
    let mut directives = CacheControl::default();
    for value in headers.get_all(CACHE_CONTROL).iter() {
        let Ok(value) = value.to_str() else {
            continue;
        };
        for directive in value.split(',') {
            let directive = directive.trim();
            if directive.eq_ignore_ascii_case("no-cache") {
                directives.no_cache = true;
            } else if directive.eq_ignore_ascii_case("no-store") {
                directives.no_store = true;
            } else if directive.eq_ignore_ascii_case("must-revalidate") {
                directives.must_revalidate = true;
            } else if let Some((name, value)) = directive.split_once('=')
                && name.trim().eq_ignore_ascii_case("max-age")
                && let Ok(seconds) = value.trim().trim_matches('"').parse::<u64>()
            {
                directives.max_age = Some(Duration::from_secs(seconds));
            }
        }
    }
    directives
}

/// Parses an HTTP-date header into seconds since the Unix epoch.
fn parse_http_date(value: Option<&reqwest::header::HeaderValue>) -> Option<Duration> {
    let value = value?.to_str().ok()?;
    httpdate::parse_http_date(value)
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
}

/// Parses the non-negative seconds in an `Age` header.
pub(super) fn parse_age(headers: &HeaderMap) -> Duration {
    headers
        .get(AGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::ZERO)
}

/// Reads an explicit `max-age` lifetime for compatibility tests.
/// Responses with `no-store` or `no-cache` have no reusable max-age lifetime.
#[cfg(test)]
pub(crate) fn cache_ttl(headers: &HeaderMap) -> Option<Duration> {
    let directives = cache_control(headers);
    (!directives.no_store && !directives.no_cache)
        .then_some(directives.max_age)
        .flatten()
}
