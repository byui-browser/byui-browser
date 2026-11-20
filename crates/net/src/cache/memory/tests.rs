use super::*;

#[test]
/// Verifies max-age is parsed alongside other cache directives.
fn cache_control_parses_max_age_with_other_directives() {
    let mut headers = HeaderMap::new();
    headers.insert(
        CACHE_CONTROL,
        "public, max-age=60, must-revalidate".parse().unwrap(),
    );

    assert_eq!(cache_ttl(&headers), Some(Duration::from_secs(60)));
}

#[test]
fn vary_header_matches_original_request() {
    let cache = ResponseCache::default();
    let url = Url::parse("https://example.test/page").unwrap();

    // Original request asks for English.
    let mut english = Request::get(url.as_str());
    english
        .set_header("Accept-Language".parse().unwrap(), "en".parse().unwrap())
        .unwrap();

    // Server says the response varies by language.
    let mut headers = HeaderMap::new();
    headers.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
    headers.insert("vary", "Accept-Language".parse().unwrap());

    let response = StoredResponse {
        status: StatusCode::OK,
        headers,
        url: url.to_string(),
        body: Bytes::from_static(b"Hello"),
        cookie_headers_processed: false,
    };

    cache.insert_with_headers(&english, &url, english.headers(), response);

    // Same language should produce a cache hit.
    assert!(fresh_response(&cache, &english, &url).is_some());

    // Different language should produce a cache miss.
    let mut spanish = Request::get(url.as_str());
    spanish
        .set_header("Accept-Language".parse().unwrap(), "es".parse().unwrap())
        .unwrap();

    assert!(fresh_response(&cache, &spanish, &url).is_none());
}

#[test]
fn vary_star_responses_are_not_cached() {
    let cache = ResponseCache::default();
    let url = Url::parse("https://example.test/page").unwrap();
    let request = Request::get(url.as_str());

    let mut headers = HeaderMap::new();
    headers.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
    headers.insert("vary", "*".parse().unwrap());

    let response = StoredResponse {
        status: StatusCode::OK,
        headers,
        url: url.to_string(),
        body: Bytes::from_static(b"Hello"),
        cookie_headers_processed: false,
    };

    cache.insert_with_headers(&request, &url, request.headers(), response);

    // Vary: * responses must not be stored.
    assert!(fresh_response(&cache, &request, &url).is_none());
}

#[test]
/// Verifies non-cacheable directives prevent insertion.
fn cache_control_rejects_non_cacheable_directives() {
    for value in ["no-store", "public, no-cache, max-age=60"] {
        let mut headers = HeaderMap::new();
        headers.insert(CACHE_CONTROL, value.parse().unwrap());
        assert_eq!(cache_ttl(&headers), None, "directive: {value}");
    }
}

#[test]
/// Verifies directive names and quoted max-age values are accepted.
fn cache_control_accepts_case_insensitive_and_quoted_max_age() {
    let mut headers = HeaderMap::new();
    headers.insert(
        CACHE_CONTROL,
        "PUBLIC, MAX-AGE = \"60\", must-revalidate".parse().unwrap(),
    );

    assert_eq!(cache_ttl(&headers), Some(Duration::from_secs(60)));
}

#[test]
fn freshness_uses_age_and_resident_time() {
    let date = httpdate::parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT")
        .unwrap()
        .duration_since(UNIX_EPOCH)
        .unwrap();
    let clock = Arc::new(TestClock::new(date + Duration::from_secs(10)));
    let cache = ResponseCache::with_clock(4, 64, clock.clone());
    let url = Url::parse("https://example.test/freshness").unwrap();
    let request = Request::get(url.as_str());
    let mut headers = HeaderMap::new();
    headers.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
    headers.insert(DATE, "Sun, 06 Nov 1994 08:49:37 GMT".parse().unwrap());
    headers.insert(AGE, "30".parse().unwrap());

    cache.insert_with_headers(
        &request,
        &url,
        request.headers(),
        StoredResponse {
            status: StatusCode::OK,
            headers,
            url: url.to_string(),
            body: Bytes::from_static(b"fresh"),
            cookie_headers_processed: false,
        },
    );
    clock.advance(Duration::from_secs(29));
    assert!(matches!(
        cache.lookup_with_headers(&request, &url, request.headers()),
        Some(CacheLookup::Fresh(_))
    ));

    clock.advance(Duration::from_secs(1));
    assert!(matches!(
        cache.lookup_with_headers(&request, &url, request.headers()),
        Some(CacheLookup::Stale { .. })
    ));
}

#[test]
fn expires_is_used_when_max_age_is_absent() {
    let date = httpdate::parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT")
        .unwrap()
        .duration_since(UNIX_EPOCH)
        .unwrap();
    let clock = Arc::new(TestClock::new(date));
    let cache = ResponseCache::with_clock(4, 64, clock.clone());
    let url = Url::parse("https://example.test/expires").unwrap();
    let request = Request::get(url.as_str());
    let mut headers = HeaderMap::new();
    headers.insert(DATE, "Sun, 06 Nov 1994 08:49:37 GMT".parse().unwrap());
    headers.insert(EXPIRES, "Sun, 06 Nov 1994 08:50:37 GMT".parse().unwrap());

    cache.insert_with_headers(
        &request,
        &url,
        request.headers(),
        StoredResponse {
            status: StatusCode::OK,
            headers,
            url: url.to_string(),
            body: Bytes::from_static(b"expires"),
            cookie_headers_processed: false,
        },
    );
    clock.advance(Duration::from_secs(59));
    assert!(matches!(
        cache.lookup_with_headers(&request, &url, request.headers()),
        Some(CacheLookup::Fresh(_))
    ));
    clock.advance(Duration::from_secs(1));
    assert!(matches!(
        cache.lookup_with_headers(&request, &url, request.headers()),
        Some(CacheLookup::Stale { .. })
    ));
}

#[test]
fn last_modified_supplies_conservative_heuristic_freshness() {
    let date = httpdate::parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT")
        .unwrap()
        .duration_since(UNIX_EPOCH)
        .unwrap();
    let clock = Arc::new(TestClock::new(date));
    let cache = ResponseCache::with_clock(4, 64, clock.clone());
    let url = Url::parse("https://example.test/heuristic").unwrap();
    let request = Request::get(url.as_str());
    let mut headers = HeaderMap::new();
    headers.insert(DATE, "Sun, 06 Nov 1994 08:49:37 GMT".parse().unwrap());
    headers.insert(
        LAST_MODIFIED,
        "Sun, 06 Nov 1994 08:39:37 GMT".parse().unwrap(),
    );

    cache.insert_with_headers(
        &request,
        &url,
        request.headers(),
        StoredResponse {
            status: StatusCode::OK,
            headers,
            url: url.to_string(),
            body: Bytes::from_static(b"heuristic"),
            cookie_headers_processed: false,
        },
    );

    clock.advance(Duration::from_secs(59));
    assert!(matches!(
        cache.lookup_with_headers(&request, &url, request.headers()),
        Some(CacheLookup::Fresh(_))
    ));
    clock.advance(Duration::from_secs(1));
    assert!(matches!(
        cache.lookup_with_headers(&request, &url, request.headers()),
        Some(CacheLookup::Stale { .. })
    ));
}

#[test]
fn explicit_freshness_overrides_last_modified_heuristics() {
    let now = CacheTime {
        wall: Duration::from_secs(1_000),
        elapsed: Duration::ZERO,
    };
    let mut headers = HeaderMap::new();
    headers.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
    headers.insert(
        LAST_MODIFIED,
        "Sun, 06 Nov 1994 08:39:37 GMT".parse().unwrap(),
    );

    let freshness = FreshnessMetadata::from_headers(&headers, now).unwrap();
    assert_eq!(freshness.lifetime, Duration::from_secs(60));
}

#[test]
fn freshness_extensions_keep_stale_entries_only_within_their_windows() {
    let now = CacheTime {
        wall: Duration::from_secs(1_000),
        elapsed: Duration::ZERO,
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        CACHE_CONTROL,
        "public, private, immutable, max-age=10, s-maxage=1, stale-while-revalidate=5, stale-if-error=20"
            .parse()
            .unwrap(),
    );
    let freshness = FreshnessMetadata::from_headers(&headers, now).unwrap();

    // This is a private browser cache, so s-maxage does not replace max-age.
    assert_eq!(freshness.lifetime, Duration::from_secs(10));
    assert!(freshness.is_fresh(CacheTime {
        wall: now.wall + Duration::from_secs(9),
        elapsed: Duration::from_secs(9),
    }));
    let stale = CacheTime {
        wall: now.wall + Duration::from_secs(11),
        elapsed: Duration::from_secs(11),
    };
    assert!(freshness.allows_stale_while_revalidate(stale));
    assert!(freshness.allows_stale_if_error(stale));
    assert!(freshness.retains_stale_representation(stale));
    let only_if_error = CacheTime {
        wall: now.wall + Duration::from_secs(16),
        elapsed: Duration::from_secs(16),
    };
    assert!(!freshness.allows_stale_while_revalidate(only_if_error));
    assert!(freshness.allows_stale_if_error(only_if_error));
    let expired = CacheTime {
        wall: now.wall + Duration::from_secs(31),
        elapsed: Duration::from_secs(31),
    };
    assert!(!freshness.retains_stale_representation(expired));
}

#[test]
fn must_revalidate_disables_stale_extensions() {
    let now = CacheTime {
        wall: Duration::ZERO,
        elapsed: Duration::ZERO,
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        CACHE_CONTROL,
        "max-age=0, must-revalidate, stale-while-revalidate=60, stale-if-error=60"
            .parse()
            .unwrap(),
    );
    let freshness = FreshnessMetadata::from_headers(&headers, now).unwrap();
    let stale = CacheTime {
        wall: Duration::from_secs(1),
        elapsed: Duration::from_secs(1),
    };
    assert!(!freshness.allows_stale_while_revalidate(stale));
    assert!(!freshness.allows_stale_if_error(stale));
}

#[test]
fn background_revalidation_leases_coalesce_matching_variants() {
    let cache = ResponseCache::default();
    let url = Url::parse("https://example.test/revalidate-once").unwrap();
    let request = Request::get(url.as_str());
    cache.insert_with_headers(
        &request,
        &url,
        request.headers(),
        cached_response(&url, b"cached"),
    );

    let lease = cache
        .begin_background_revalidation(&request, &url, request.headers())
        .expect("first refresh should acquire the lease");
    assert!(
        cache
            .begin_background_revalidation(&request, &url, request.headers())
            .is_none()
    );
    drop(lease);
    assert!(
        cache
            .begin_background_revalidation(&request, &url, request.headers())
            .is_some()
    );
}

#[test]
fn no_cache_and_must_revalidate_entries_require_revalidation_when_stale() {
    let clock = Arc::new(TestClock::new(Duration::ZERO));
    let cache = ResponseCache::with_clock(4, 64, clock);
    let url = Url::parse("https://example.test/revalidate").unwrap();
    let request = Request::get(url.as_str());

    let mut no_cache_headers = HeaderMap::new();
    no_cache_headers.insert(CACHE_CONTROL, "no-cache, max-age=60".parse().unwrap());
    cache.insert_with_headers(
        &request,
        &url,
        request.headers(),
        StoredResponse {
            status: StatusCode::OK,
            headers: no_cache_headers,
            url: url.to_string(),
            body: Bytes::from_static(b"no-cache"),
            cookie_headers_processed: false,
        },
    );
    assert!(matches!(
        cache.lookup_with_headers(&request, &url, request.headers()),
        Some(CacheLookup::Stale { .. })
    ));

    let mut must_revalidate_headers = HeaderMap::new();
    must_revalidate_headers.insert(CACHE_CONTROL, "max-age=0, must-revalidate".parse().unwrap());
    cache.insert_with_headers(
        &request,
        &url,
        request.headers(),
        StoredResponse {
            status: StatusCode::OK,
            headers: must_revalidate_headers,
            url: url.to_string(),
            body: Bytes::from_static(b"must-revalidate"),
            cookie_headers_processed: false,
        },
    );
    assert!(matches!(
        cache.lookup_with_headers(&request, &url, request.headers()),
        Some(CacheLookup::Stale { .. })
    ));
}

#[test]
fn matching_lookup_returns_entries_that_normally_require_revalidation() {
    let clock = Arc::new(TestClock::new(Duration::ZERO));
    let cache = ResponseCache::with_clock(4, 64, clock);
    let url = Url::parse("https://example.test/revalidation-restricted").unwrap();
    let request = Request::get(url.as_str());

    for (directives, body) in [
        ("no-cache, max-age=60", b"no-cache".as_slice()),
        ("max-age=0, must-revalidate", b"must-revalidate".as_slice()),
    ] {
        let mut headers = HeaderMap::new();
        headers.insert(CACHE_CONTROL, directives.parse().unwrap());
        cache.insert_with_headers(
            &request,
            &url,
            request.headers(),
            StoredResponse {
                status: StatusCode::OK,
                headers,
                url: url.to_string(),
                body: Bytes::copy_from_slice(body),
                cookie_headers_processed: false,
            },
        );

        assert!(matches!(
            cache.lookup_with_headers(&request, &url, request.headers()),
            Some(CacheLookup::Stale { .. })
        ));
        assert_eq!(
            cache
                .lookup_matching_with_headers(&request, &url, request.headers())
                .unwrap()
                .body
                .as_ref(),
            body
        );
    }
}

#[test]
fn revalidation_sends_all_validators_and_merges_304_metadata() {
    let clock = Arc::new(TestClock::new(Duration::ZERO));
    let cache = ResponseCache::with_clock(4, 64, clock);
    let url = Url::parse("https://example.test/revalidate").unwrap();
    let request = Request::get(url.as_str());
    let mut headers = HeaderMap::new();
    headers.insert(CACHE_CONTROL, "max-age=0".parse().unwrap());
    headers.insert(ETAG, "\"first\"".parse().unwrap());
    headers.insert(
        LAST_MODIFIED,
        "Sun, 06 Nov 1994 08:49:37 GMT".parse().unwrap(),
    );
    headers.append(
        http::header::WARNING,
        "110 example.test \"stale\"".parse().unwrap(),
    );
    headers.append(
        http::header::WARNING,
        "214 example.test \"transformed\"".parse().unwrap(),
    );
    cache.insert_with_headers(
        &request,
        &url,
        request.headers(),
        StoredResponse {
            status: StatusCode::OK,
            headers,
            url: url.to_string(),
            body: Bytes::from_static(b"cached"),
            cookie_headers_processed: false,
        },
    );

    let (stored, validator) = cache
        .lookup_for_revalidation_with_headers(&request, &url, request.headers())
        .unwrap();
    let mut conditional_headers = crate::Headers::new();
    validator.apply(&mut conditional_headers).unwrap();
    assert_eq!(
        conditional_headers
            .to_reqwest()
            .get("if-none-match")
            .unwrap(),
        "\"first\""
    );
    assert_eq!(
        conditional_headers
            .to_reqwest()
            .get("if-modified-since")
            .unwrap(),
        "Sun, 06 Nov 1994 08:49:37 GMT"
    );

    let mut not_modified = HeaderMap::new();
    not_modified.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
    not_modified.insert(ETAG, "\"second\"".parse().unwrap());
    not_modified.insert("x-revalidated", "yes".parse().unwrap());
    not_modified.insert("set-cookie", "session=updated".parse().unwrap());
    let merged = cache
        .merge_not_modified_with_headers(&request, &url, request.headers(), &stored, &not_modified)
        .unwrap();
    assert_eq!(merged.status, StatusCode::OK);
    assert_eq!(merged.body, Bytes::from_static(b"cached"));
    assert_eq!(merged.headers.get("x-revalidated").unwrap(), "yes");
    assert!(merged.headers.get("set-cookie").is_none());
    let warnings: Vec<_> = merged
        .headers
        .get_all(http::header::WARNING)
        .iter()
        .collect();
    assert_eq!(warnings, ["214 example.test \"transformed\""]);
    let (_, refreshed_validator) = cache
        .lookup_for_revalidation_with_headers(&request, &url, request.headers())
        .unwrap();
    let mut refreshed_headers = crate::Headers::new();
    refreshed_validator.apply(&mut refreshed_headers).unwrap();
    assert_eq!(
        refreshed_headers.to_reqwest().get("if-none-match").unwrap(),
        "\"second\""
    );
    assert!(matches!(
        cache.lookup_with_headers(&request, &url, request.headers()),
        Some(CacheLookup::Fresh(_))
    ));
}

#[test]
fn late_304_does_not_freshen_a_concurrently_replaced_representation() {
    let cache = ResponseCache::default();
    let url = Url::parse("https://example.test/concurrent-revalidation").unwrap();
    let request = Request::get(url.as_str());
    let mut original_headers = HeaderMap::new();
    original_headers.insert(CACHE_CONTROL, "max-age=0".parse().unwrap());
    original_headers.insert(ETAG, "\"old\"".parse().unwrap());
    cache.insert_with_headers(
        &request,
        &url,
        request.headers(),
        StoredResponse {
            status: StatusCode::OK,
            headers: original_headers,
            url: url.to_string(),
            body: Bytes::from_static(b"old"),
            cookie_headers_processed: false,
        },
    );
    let (validated, _) = cache
        .lookup_for_revalidation_with_headers(&request, &url, request.headers())
        .unwrap();

    let mut replacement_headers = HeaderMap::new();
    replacement_headers.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
    replacement_headers.insert(ETAG, "\"new\"".parse().unwrap());
    cache.insert_with_headers(
        &request,
        &url,
        request.headers(),
        StoredResponse {
            status: StatusCode::OK,
            headers: replacement_headers,
            url: url.to_string(),
            body: Bytes::from_static(b"new"),
            cookie_headers_processed: false,
        },
    );

    let mut late_not_modified = HeaderMap::new();
    late_not_modified.insert(CACHE_CONTROL, "max-age=600".parse().unwrap());
    late_not_modified.insert("x-late-validation", "old".parse().unwrap());
    assert!(
        cache
            .merge_not_modified_with_headers(
                &request,
                &url,
                request.headers(),
                &validated,
                &late_not_modified,
            )
            .is_none()
    );

    let cached = cache
        .lookup_matching_with_headers(&request, &url, request.headers())
        .unwrap();
    assert_eq!(cached.body, Bytes::from_static(b"new"));
    assert_eq!(cached.headers.get(ETAG).unwrap(), "\"new\"");
    assert!(cached.headers.get("x-late-validation").is_none());
}

#[test]
fn revalidation_updates_vary_snapshot_when_304_changes_vary_fields() {
    let clock = Arc::new(TestClock::new(Duration::ZERO));
    let cache = ResponseCache::with_clock(4, 64, clock);
    let url = Url::parse("https://example.test/revalidate-vary").unwrap();
    let mut original = Request::get(url.as_str());
    original
        .set_header("Accept-Language".parse().unwrap(), "en".parse().unwrap())
        .unwrap();
    original
        .set_header(
            "User-Agent".parse().unwrap(),
            "first-agent".parse().unwrap(),
        )
        .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(CACHE_CONTROL, "max-age=0".parse().unwrap());
    headers.insert(ETAG, "\"first\"".parse().unwrap());
    headers.insert("vary", "Accept-Language".parse().unwrap());
    cache.insert_with_headers(
        &original,
        &url,
        original.headers(),
        StoredResponse {
            status: StatusCode::OK,
            headers,
            url: url.to_string(),
            body: Bytes::from_static(b"cached"),
            cookie_headers_processed: false,
        },
    );

    let mut not_modified = HeaderMap::new();
    not_modified.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
    not_modified.insert("vary", "User-Agent".parse().unwrap());
    let (stored, _) = cache
        .lookup_for_revalidation_with_headers(&original, &url, original.headers())
        .unwrap();
    cache
        .merge_not_modified_with_headers(
            &original,
            &url,
            original.headers(),
            &stored,
            &not_modified,
        )
        .unwrap();

    let mut same_agent = Request::get(url.as_str());
    same_agent
        .set_header("Accept-Language".parse().unwrap(), "fr".parse().unwrap())
        .unwrap();
    same_agent
        .set_header(
            "User-Agent".parse().unwrap(),
            "first-agent".parse().unwrap(),
        )
        .unwrap();
    assert!(matches!(
        cache.lookup_with_headers(&same_agent, &url, same_agent.headers()),
        Some(CacheLookup::Fresh(_))
    ));

    let mut different_agent = same_agent.clone_without_body();
    different_agent
        .set_header(
            "User-Agent".parse().unwrap(),
            "second-agent".parse().unwrap(),
        )
        .unwrap();
    assert!(
        cache
            .lookup_with_headers(&different_agent, &url, different_agent.headers())
            .is_none()
    );
}

#[test]
fn malformed_expiry_and_negative_age_do_not_extend_freshness() {
    let now = CacheTime {
        wall: Duration::from_secs(1_000),
        elapsed: Duration::ZERO,
    };
    let mut invalid_expiry = HeaderMap::new();
    invalid_expiry.insert(EXPIRES, "not a date".parse().unwrap());
    assert!(FreshnessMetadata::from_headers(&invalid_expiry, now).is_none());

    let mut negative_age = HeaderMap::new();
    negative_age.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
    negative_age.insert(AGE, "-1".parse().unwrap());
    assert_eq!(parse_age(&negative_age), Duration::ZERO);
    assert!(
        FreshnessMetadata::from_headers(&negative_age, now)
            .unwrap()
            .is_fresh(now)
    );
}

#[test]
/// Verifies default-cacheable error responses enter the cache.
fn cache_stores_default_cacheable_error_responses() {
    let cache = ResponseCache::default();
    let request = Request::get("https://example.test/missing");
    let mut headers = HeaderMap::new();
    headers.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
    let response = StoredResponse {
        status: StatusCode::NOT_FOUND,
        headers,
        url: request.url().to_owned(),
        body: Bytes::from_static(b"missing"),
        cookie_headers_processed: false,
    };

    let url = Url::parse(request.url()).unwrap();
    cache.insert_with_headers(&request, &url, request.headers(), response);

    assert_eq!(
        fresh_response(&cache, &request, &url).unwrap().status,
        StatusCode::NOT_FOUND
    );
}

#[test]
/// Verifies temporary redirects need explicit cache metadata.
fn cache_stores_temporary_redirects_only_with_explicit_metadata() {
    let cache = ResponseCache::default();
    let request = Request::get("https://example.test/redirect");
    let url = Url::parse(request.url()).unwrap();

    let no_metadata = StoredResponse {
        status: StatusCode::FOUND,
        headers: HeaderMap::new(),
        url: request.url().to_owned(),
        body: Bytes::new(),
        cookie_headers_processed: false,
    };
    cache.insert_with_headers(&request, &url, request.headers(), no_metadata);
    assert!(fresh_response(&cache, &request, &url).is_none());

    let mut headers = HeaderMap::new();
    headers.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
    let explicit_metadata = StoredResponse {
        status: StatusCode::FOUND,
        headers,
        url: request.url().to_owned(),
        body: Bytes::new(),
        cookie_headers_processed: false,
    };
    cache.insert_with_headers(&request, &url, request.headers(), explicit_metadata);
    assert_eq!(
        fresh_response(&cache, &request, &url).unwrap().status,
        StatusCode::FOUND
    );
}

#[test]
/// Verifies cache keys distinguish request methods.
fn cache_key_distinguishes_http_methods() {
    let get = Request::get("https://example.test/resource");
    let mut head = get.try_clone().unwrap();
    head.set_method("HEAD").unwrap();

    let url = Url::parse(get.url()).unwrap();
    assert_ne!(cache_key(&get, &url), cache_key(&head, &url));
}

#[test]
/// Verifies browser cache partitions remain isolated.
fn browser_cache_partitions_do_not_share_entries() {
    let url = Url::parse("https://example.test/resource").unwrap();
    let mut first = Request::get(url.as_str());
    first.set_cache_partition("site-a".into());
    let mut second = first.try_clone().unwrap();
    second.set_cache_partition("site-b".into());
    assert_ne!(cache_key(&first, &url), cache_key(&second, &url));
    assert_ne!(
        cache_key(&first, &url),
        cache_key(&Request::get(url.as_str()), &url)
    );
}

#[test]
/// Verifies URL fragments do not affect cache identity.
fn cache_key_ignores_url_fragments() {
    let with_fragment = Request::get("https://example.test/resource#section");
    let without_fragment = Request::get("https://example.test/resource");
    let with_fragment_url = Url::parse(with_fragment.url()).unwrap();
    let without_fragment_url = Url::parse(without_fragment.url()).unwrap();

    assert_eq!(
        cache_key(&with_fragment, &with_fragment_url),
        cache_key(&without_fragment, &without_fragment_url)
    );
}

#[test]
/// Verifies equivalent normalized URLs share a cache key.
fn cache_key_uses_normalized_url_strings() {
    let uppercase = Request::get("HTTP://EXAMPLE.TEST/resource");
    let normalized = Request::get("http://example.test/resource");
    let uppercase_url = Url::parse(uppercase.url()).unwrap();
    let normalized_url = Url::parse(normalized.url()).unwrap();

    assert_eq!(
        cache_key(&uppercase, &uppercase_url),
        cache_key(&normalized, &normalized_url)
    );
}

#[test]
/// Verifies expired entries are treated as cache misses.
fn expired_entries_are_not_returned() {
    let cache = ResponseCache::default();
    let request = Request::get("https://example.test/expired");
    let response = StoredResponse {
        status: StatusCode::OK,
        headers: HeaderMap::new(),
        url: request.url().to_owned(),
        body: Bytes::from_static(b"stale"),
        cookie_headers_processed: false,
    };
    let cached = CachedResponse::from_response(request.headers(), response, expired_freshness(), 0);

    cache.entries.write().unwrap().entries.insert(
        cache_key(&request, &Url::parse(request.url()).unwrap()),
        vec![cached],
    );

    assert!(matches!(
        cache.lookup_with_headers(
            &request,
            &Url::parse(request.url()).unwrap(),
            request.headers(),
        ),
        Some(CacheLookup::Stale { .. })
    ));
}

#[test]
fn expired_entries_are_not_returned2() {
    let cache = ResponseCache::default();
    let request = Request::get("https://example.test/expired2");
    let response = StoredResponse {
        status: StatusCode::FOUND,
        headers: HeaderMap::new(),
        url: request.url().to_owned(),
        body: Bytes::from_static(b"stale"),
        cookie_headers_processed: false,
    };
    let cached = CachedResponse::from_response(request.headers(), response, expired_freshness(), 0);

    cache.entries.write().unwrap().entries.insert(
        cache_key(&request, &Url::parse(request.url()).unwrap()),
        vec![cached],
    );

    assert!(matches!(
        cache.lookup_with_headers(
            &request,
            &Url::parse(request.url()).unwrap(),
            request.headers(),
        ),
        Some(CacheLookup::Stale { .. })
    ));
}

#[test]
fn multiple_vary_variants_are_cached() {
    let cache = ResponseCache::default();
    let url = Url::parse("https://example.test/page").unwrap();

    // Create requests for two different languages.
    let mut english = Request::get(url.as_str());
    english
        .set_header("Accept-Language".parse().unwrap(), "en".parse().unwrap())
        .unwrap();

    let mut spanish = Request::get(url.as_str());
    spanish
        .set_header("Accept-Language".parse().unwrap(), "es".parse().unwrap())
        .unwrap();

    // Both responses vary by the Accept-Language header.
    let mut headers = HeaderMap::new();
    headers.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
    headers.insert("vary", "Accept-Language".parse().unwrap());

    let english_response = StoredResponse {
        status: StatusCode::OK,
        headers: headers.clone(),
        url: url.to_string(),
        body: Bytes::from_static(b"Hello"),
        cookie_headers_processed: false,
    };

    let spanish_response = StoredResponse {
        status: StatusCode::OK,
        headers,
        url: url.to_string(),
        body: Bytes::from_static(b"Hola"),
        cookie_headers_processed: false,
    };

    // Store both responses under the same URL.
    cache.insert_with_headers(&english, &url, english.headers(), english_response.clone());
    cache.insert_with_headers(&spanish, &url, spanish.headers(), spanish_response);

    // Each language should retrieve its own response.
    assert_eq!(
        fresh_response(&cache, &english, &url)
            .unwrap()
            .body
            .as_ref(),
        b"Hello"
    );
    assert_eq!(
        fresh_response(&cache, &spanish, &url)
            .unwrap()
            .body
            .as_ref(),
        b"Hola"
    );

    // Update the English response with new content.
    let updated_english_response = StoredResponse {
        body: Bytes::from_static(b"Hello again"),
        ..english_response.clone()
    };

    cache.insert_with_headers(&english, &url, english.headers(), updated_english_response);

    // English should return the updated response.
    assert_eq!(
        fresh_response(&cache, &english, &url)
            .unwrap()
            .body
            .as_ref(),
        b"Hello again"
    );

    // Spanish should still return its original response.
    assert_eq!(
        fresh_response(&cache, &spanish, &url)
            .unwrap()
            .body
            .as_ref(),
        b"Hola"
    );
}

#[test]
fn vary_header_order_does_not_matter() {
    let mut first = HeaderMap::new();
    first.insert("vary", "Accept-Language, Accept-Encoding".parse().unwrap());

    let mut second = HeaderMap::new();
    second.insert("vary", "Accept-Encoding, Accept-Language".parse().unwrap());

    assert_eq!(vary_header_names(&first), vary_header_names(&second));
}

#[test]
fn changing_vary_rules_removes_old_variants() {
    let cache = ResponseCache::default();
    let url = Url::parse("https://example.test/page").unwrap();

    let mut request = Request::get(url.as_str());
    request
        .set_header("Accept-Language".parse().unwrap(), "en".parse().unwrap())
        .unwrap();
    request
        .set_header("Accept".parse().unwrap(), "text/html".parse().unwrap())
        .unwrap();

    let mut language_headers = HeaderMap::new();
    language_headers.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
    language_headers.insert("vary", "Accept-Language".parse().unwrap());

    let language_response = StoredResponse {
        status: StatusCode::OK,
        headers: language_headers,
        url: url.to_string(),
        body: Bytes::from_static(b"English response"),
        cookie_headers_processed: false,
    };

    cache.insert_with_headers(&request, &url, request.headers(), language_response.clone());

    let mut encoding_headers = HeaderMap::new();
    encoding_headers.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
    encoding_headers.insert("vary", "Accept".parse().unwrap());

    let encoding_response = StoredResponse {
        headers: encoding_headers,
        body: Bytes::from_static(b"Gzip response"),
        ..language_response.clone()
    };

    cache.insert_with_headers(&request, &url, request.headers(), encoding_response);

    // The cache should contain only the new Vary rule.
    let key = cache_key(&request, &url);
    let entries = cache.entries.read().unwrap();
    let variants = entries.entries.get(&key).unwrap();

    assert_eq!(variants.len(), 1);
    assert_eq!(variants[0].response.body.as_ref(), b"Gzip response");
}

#[test]
/// Verifies a cache hit shares immutable body storage with its cache entry.
fn cache_hits_share_immutable_body_bytes() {
    let cache = ResponseCache::default();
    let request = Request::get("https://example.test/shared-body");
    let url = Url::parse(request.url()).unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());

    cache.insert_with_headers(
        &request,
        &url,
        request.headers(),
        StoredResponse {
            status: StatusCode::OK,
            headers,
            url: url.to_string(),
            body: Bytes::from(vec![1_u8; 1024]),
            cookie_headers_processed: false,
        },
    );

    let hit = fresh_response(&cache, &request, &url).unwrap();
    let entries = cache.entries.read().unwrap();
    let cached = &entries.entries[&cache_key(&request, &url)][0];

    assert_eq!(hit.body.as_ref(), cached.response.body.as_ref());
    assert_eq!(hit.body.as_ptr(), cached.response.body.as_ptr());
}

#[test]
fn cache_keys_isolate_credentials_modes_request_modes_and_authorization() {
    let url = Url::parse("https://example.test/identity").unwrap();
    let omit = Request::get(url.as_str());

    let mut include = omit.try_clone().unwrap();
    include.set_credentials_mode(CredentialsMode::Include);
    assert_ne!(cache_key(&omit, &url), cache_key(&include, &url));

    let mut no_cors = omit.try_clone().unwrap();
    no_cors.set_mode(RequestMode::NoCors).unwrap();
    assert_ne!(cache_key(&omit, &url), cache_key(&no_cors, &url));

    let mut authorized = omit.try_clone().unwrap();
    authorized
        .set_header(AUTHORIZATION, "Bearer cache-key-test".parse().unwrap())
        .unwrap();
    assert_ne!(cache_key(&omit, &url), cache_key(&authorized, &url));
}

#[test]
fn cache_evicts_the_least_recently_used_primary_entry() {
    let cache = ResponseCache::new(2, 64);
    let first = Url::parse("https://example.test/first").unwrap();
    let second = Url::parse("https://example.test/second").unwrap();
    let third = Url::parse("https://example.test/third").unwrap();
    let first_request = Request::get(first.as_str());
    let second_request = Request::get(second.as_str());
    let third_request = Request::get(third.as_str());

    cache.insert_with_headers(
        &first_request,
        &first,
        first_request.headers(),
        cached_response(&first, b"first"),
    );
    cache.insert_with_headers(
        &second_request,
        &second,
        second_request.headers(),
        cached_response(&second, b"second"),
    );
    assert!(fresh_response(&cache, &first_request, &first).is_some());
    cache.insert_with_headers(
        &third_request,
        &third,
        third_request.headers(),
        cached_response(&third, b"third"),
    );

    assert!(fresh_response(&cache, &first_request, &first).is_some());
    assert!(fresh_response(&cache, &second_request, &second).is_none());
    assert!(fresh_response(&cache, &third_request, &third).is_some());
}

#[test]
fn cache_evicts_across_partitions_to_stay_within_the_body_budget() {
    let cache = ResponseCache::new(4, 8);
    let url = Url::parse("https://example.test/budget").unwrap();
    let mut first = Request::get(url.as_str());
    first.set_cache_partition("one".into());
    let mut second = first.try_clone().unwrap();
    second.set_cache_partition("two".into());

    cache.insert_with_headers(
        &first,
        &url,
        first.headers(),
        cached_response(&url, b"first"),
    );
    cache.insert_with_headers(
        &second,
        &url,
        second.headers(),
        cached_response(&url, b"second"),
    );

    assert!(fresh_response(&cache, &first, &url).is_none());
    assert!(fresh_response(&cache, &second, &url).is_some());
    assert_eq!(cache.entries.read().unwrap().total_body_size, 6);
}

#[test]
fn insertion_removes_expired_entries_before_admitting_new_data() {
    let cache = ResponseCache::new(4, 64);
    let expired_url = Url::parse("https://example.test/expired-prune").unwrap();
    let fresh_url = Url::parse("https://example.test/fresh-prune").unwrap();
    let expired_request = Request::get(expired_url.as_str());
    let fresh_request = Request::get(fresh_url.as_str());
    let expired_key = cache_key(&expired_request, &expired_url);

    cache.insert_with_headers(
        &expired_request,
        &expired_url,
        expired_request.headers(),
        cached_response(&expired_url, b"expired"),
    );
    cache
        .entries
        .write()
        .unwrap()
        .entries
        .get_mut(&expired_key)
        .unwrap()[0]
        .freshness = expired_freshness();

    cache.insert_with_headers(
        &fresh_request,
        &fresh_url,
        fresh_request.headers(),
        cached_response(&fresh_url, b"fresh"),
    );

    let entries = cache.entries.read().unwrap();
    assert!(!entries.entries.contains_key(&expired_key));
    assert_eq!(entries.entries.len(), 1);
    assert_eq!(entries.total_body_size, 5);
}

#[test]
fn authorized_requests_are_never_stored() {
    let cache = ResponseCache::default();
    let url = Url::parse("https://example.test/authorized").unwrap();
    let mut request = Request::get(url.as_str());
    request
        .set_header(AUTHORIZATION, "Bearer do-not-store".parse().unwrap())
        .unwrap();

    cache.insert_with_headers(
        &request,
        &url,
        request.headers(),
        cached_response(&url, b"private"),
    );

    assert!(fresh_response(&cache, &request, &url).is_none());
}

#[test]
fn invalidation_removes_every_variant_in_its_partition_only() {
    let cache = ResponseCache::default();
    let url = Url::parse("https://example.test/articles/42#summary").unwrap();
    let other_url = Url::parse("https://example.test/articles/43").unwrap();

    let mut first = Request::get(url.as_str());
    first.set_cache_partition("first".into());
    let mut second = first.try_clone().unwrap();
    second.set_cache_partition("second".into());
    let mut mutate = first.try_clone().unwrap();
    mutate.set_method("PATCH").unwrap();

    cache.insert_with_headers(
        &first,
        &url,
        first.headers(),
        cached_response(&url, b"first"),
    );
    cache.insert_with_headers(
        &second,
        &url,
        second.headers(),
        cached_response(&url, b"second"),
    );
    cache.insert_with_headers(
        &first,
        &other_url,
        first.headers(),
        cached_response(&other_url, b"other"),
    );

    cache.invalidate(&mutate, &url);

    assert!(fresh_response(&cache, &first, &url).is_none());
    assert_eq!(
        fresh_response(&cache, &second, &url).unwrap().body.as_ref(),
        b"second"
    );
    assert_eq!(
        fresh_response(&cache, &first, &other_url)
            .unwrap()
            .body
            .as_ref(),
        b"other"
    );
}

fn cached_response(url: &Url, body: &'static [u8]) -> StoredResponse {
    let mut headers = HeaderMap::new();
    headers.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
    StoredResponse {
        status: StatusCode::OK,
        headers,
        url: url.to_string(),
        body: Bytes::from_static(body),
        cookie_headers_processed: false,
    }
}

fn fresh_response(cache: &ResponseCache, request: &Request, url: &Url) -> Option<StoredResponse> {
    match cache.lookup_with_headers(request, url, request.headers()) {
        Some(CacheLookup::Fresh(response)) => Some(response),
        Some(CacheLookup::Stale { .. }) | None => None,
    }
}

fn expired_freshness() -> FreshnessMetadata {
    FreshnessMetadata {
        response_time: CacheTime {
            wall: Duration::ZERO,
            elapsed: Duration::ZERO,
        },
        date: None,
        age: Duration::ZERO,
        lifetime: Duration::ZERO,
        requires_revalidation: false,
        stale_while_revalidate: Duration::ZERO,
        stale_if_error: Duration::ZERO,
        forbids_stale: false,
    }
}

#[derive(Debug)]
struct TestClock {
    now: RwLock<CacheTime>,
}

impl TestClock {
    fn new(wall: Duration) -> Self {
        Self {
            now: RwLock::new(CacheTime {
                wall,
                elapsed: Duration::ZERO,
            }),
        }
    }

    fn advance(&self, duration: Duration) {
        let mut now = self.now.write().unwrap();
        now.wall = now.wall.saturating_add(duration);
        now.elapsed = now.elapsed.saturating_add(duration);
    }
}

impl CacheClock for TestClock {
    fn now(&self) -> CacheTime {
        *self.now.read().unwrap()
    }
}
