//! In-memory response cache and cache-policy helpers.

mod cache;
mod entry;
mod freshness;
mod key;
mod storage;

pub(crate) use cache::{CacheLookup, ResponseCache};
pub(crate) use entry::StoredResponse;

#[cfg(test)]
use std::{
    sync::{Arc, RwLock},
    time::{Duration, UNIX_EPOCH},
};

#[cfg(test)]
use bytes::Bytes;
#[cfg(test)]
use reqwest::{
    StatusCode, Url,
    header::{AGE, AUTHORIZATION, CACHE_CONTROL, DATE, EXPIRES, HeaderMap},
};

#[cfg(test)]
use crate::api::request::{CredentialsMode, Request, RequestMode};
#[cfg(test)]
use entry::{CachedResponse, vary_header_names};
#[cfg(test)]
use freshness::{CacheClock, CacheTime, FreshnessMetadata, cache_ttl, parse_age};
#[cfg(test)]
use key::cache_key;

#[cfg(test)]
#[path = "memory/tests.rs"]
mod tests;
