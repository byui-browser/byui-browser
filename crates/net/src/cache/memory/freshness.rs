//! HTTP response freshness policy and time sources.

use std::{
    fmt,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use reqwest::header::{AGE, CACHE_CONTROL, DATE, EXPIRES, HeaderMap, LAST_MODIFIED};

/// The conventional fraction of a resource's age used for heuristic freshness.
///
/// RFC 9111 leaves the precise fraction to the cache implementation. Ten
/// percent is the interoperable convention and keeps a response without
/// explicit cache directives from remaining fresh for its entire apparent
/// lifetime.
const HEURISTIC_FRESHNESS_FRACTION: u32 = 10;

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
    s_maxage: Option<Duration>,
    stale_while_revalidate: Option<Duration>,
    stale_if_error: Option<Duration>,
    no_cache: bool,
    no_store: bool,
    must_revalidate: bool,
    immutable: bool,
    visibility: CacheVisibility,
}

/// Visibility directives understood by the private browser cache.
///
/// `public` and `private` constrain shared caches. This process-local cache is
/// private to its browser partition, so it records both directives for
/// diagnostics and future durable backends without changing admission.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum CacheVisibility {
    #[default]
    Unspecified,
    Public,
    Private,
}

/// Stored age and freshness state for one cached response.
#[derive(Clone, Copy, Debug)]
pub(super) struct FreshnessMetadata {
    pub(super) response_time: CacheTime,
    pub(super) date: Option<Duration>,
    pub(super) age: Duration,
    pub(super) lifetime: Duration,
    pub(super) requires_revalidation: bool,
    pub(crate) stale_while_revalidate: Duration,
    pub(crate) stale_if_error: Duration,
    pub(crate) forbids_stale: bool,
}

impl FreshnessMetadata {
    /// Builds cache metadata from explicit or heuristic HTTP freshness data.
    ///
    /// Explicit `max-age` and `Expires` values take precedence. When neither
    /// is present, a valid `Last-Modified` value supplies a conservative
    /// heuristic lifetime: one tenth of the apparent time since modification.
    /// Responses without either explicit metadata or `Last-Modified` remain
    /// ineligible for storage.
    pub(super) fn from_headers(headers: &HeaderMap, response_time: CacheTime) -> Option<Self> {
        let directives = cache_control(headers);
        if directives.no_store {
            return None;
        }
        let date = parse_http_date(headers.get(DATE));
        let expires = parse_http_date(headers.get(EXPIRES));
        let last_modified = parse_http_date(headers.get(LAST_MODIFIED));
        // `s-maxage` is deliberately ignored here: this is a private browser
        // cache, not a shared intermediary. A durable shared backend can use
        // the retained parser field when it supplies a different policy.
        let lifetime = if directives.no_cache {
            Duration::ZERO
        } else if let Some(max_age) = directives.max_age {
            max_age
        } else if let Some(expires) = expires {
            expires.saturating_sub(date.unwrap_or(response_time.wall))
        } else {
            let last_modified = last_modified?;
            let apparent_age = date
                .unwrap_or(response_time.wall)
                .saturating_sub(last_modified);
            apparent_age / HEURISTIC_FRESHNESS_FRACTION
        };
        let _visibility = directives.visibility;
        let _shared_lifetime = directives.s_maxage;
        Some(Self {
            response_time,
            date,
            age: parse_age(headers),
            lifetime,
            requires_revalidation: directives.no_cache,
            stale_while_revalidate: directives.stale_while_revalidate.unwrap_or(Duration::ZERO),
            stale_if_error: directives.stale_if_error.unwrap_or(Duration::ZERO),
            forbids_stale: directives.no_cache || directives.must_revalidate,
        })
    }

    pub(super) fn is_fresh(&self, now: CacheTime) -> bool {
        !self.requires_revalidation && self.current_age(now) < self.lifetime
    }

    /// Reports whether this stale entry may be returned while it is refreshed.
    pub(super) fn allows_stale_while_revalidate(&self, now: CacheTime) -> bool {
        !self.forbids_stale
            && self.current_age(now) < self.lifetime.saturating_add(self.stale_while_revalidate)
    }

    /// Reports whether this stale entry may mask a network or server failure.
    pub(super) fn allows_stale_if_error(&self, now: CacheTime) -> bool {
        !self.forbids_stale
            && self.current_age(now) < self.lifetime.saturating_add(self.stale_if_error)
    }

    /// Reports whether a stale entry should remain stored for an extension.
    pub(super) fn retains_stale_representation(&self, now: CacheTime) -> bool {
        self.allows_stale_while_revalidate(now) || self.allows_stale_if_error(now)
    }

    pub(super) fn current_age(&self, now: CacheTime) -> Duration {
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
            } else if directive.eq_ignore_ascii_case("immutable") {
                directives.immutable = true;
            } else if directive.eq_ignore_ascii_case("public") {
                directives.visibility = CacheVisibility::Public;
            } else if directive.eq_ignore_ascii_case("private") {
                directives.visibility = CacheVisibility::Private;
            } else if let Some((name, value)) = directive.split_once('=') {
                let Ok(seconds) = value.trim().trim_matches('"').parse::<u64>() else {
                    continue;
                };
                match name.trim().to_ascii_lowercase().as_str() {
                    "max-age" => directives.max_age = Some(Duration::from_secs(seconds)),
                    // This process-local cache is private. Keep parsing
                    // s-maxage so a future shared backend can use it, but do
                    // not apply it as a private-cache lifetime.
                    "s-maxage" => directives.s_maxage = Some(Duration::from_secs(seconds)),
                    "stale-while-revalidate" => {
                        directives.stale_while_revalidate = Some(Duration::from_secs(seconds));
                    }
                    "stale-if-error" => {
                        directives.stale_if_error = Some(Duration::from_secs(seconds));
                    }
                    _ => {}
                }
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
