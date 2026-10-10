# Net crate test plan

This document turns the current net-crate test gaps into an implementation
plan. It covers both regression tests for behavior already supported by the
current Fetch slice and conformance tests that should be added as planned
features become implemented.

The current suite covers the main request, response, cache, redirect,
streaming, and scheduler paths. The goal of this plan is not to enumerate
every possible byte sequence, but to cover each observable state transition,
security boundary, failure timing, and intentional limitation.

## Test organization

Use the existing split between unit and integration tests:

- Unit tests belong beside the implementation when they exercise parsing,
  normalization, state transitions, or pure policy decisions.
- Integration tests belong in `crates/net/tests` when they need a local TCP
  server, scheduler behavior, body streaming, service hooks, or a complete
  controller request.
- Unsupported features should have tests that prove they fail at the stated
  boundary and before network I/O. Those tests should be changed when the
  feature is implemented rather than deleted.

Suggested integration-test helpers should record request count, request
headers, request body bytes, service-hook calls, and whether a response body
was fully consumed. Existing helpers in `crates/net/tests/common` should be
extended instead of duplicating server logic.

## 1. Header guards and exposure

Primary files:

- `crates/net/src/api/headers.rs`
- `crates/net/tests/requests.rs`

### Tests to add

- `header_lookup_is_case_insensitive`: `get` and `has` find names regardless
  of casing.
- `append_preserves_duplicate_order`: duplicate values remain in insertion
  order during iteration and transport conversion.
- `insert_replaces_all_duplicate_values`: `insert` removes every existing
  value for the name and appends exactly one replacement.
- `invalid_header_name_is_rejected`: malformed names return a typed
  `ForbiddenHeader` or validation error.
- `invalid_header_value_is_rejected`: control characters and invalid bytes do
  not enter any header list.
- `request_guard_rejects_every_forbidden_header`: cover the explicit list,
  `proxy-*`, and `sec-*` names.
- `response_guard_rejects_cookie_headers`: `Set-Cookie` and `Set-Cookie2`
  cannot be added to a public response header list.
- `immutable_headers_reject_mutation`: public exposed headers reject both
  `append` and `insert`.
- `switching_to_no_cors_filters_existing_headers`: changing a request to
  `NoCors` removes headers that are not CORS-safelisted.
- `no_cors_accepts_only_safelisted_values`: cover `Accept`, language headers,
  supported `Content-Type` values, and valid `Range` syntax.
- `no_cors_rejects_long_or_control_character_values`: cover the 128-byte
  limit and forbidden control-character ranges.
- `no_cors_validates_content_type_media_parameters`: accept supported media
  types with parameters and reject unsupported types.
- `no_cors_validates_range_grammar`: cover open-ended ranges, numeric ranges,
  missing bounds, multiple ranges, signs, whitespace, and nonnumeric input.
- `cors_exposure_is_case_insensitive_and_ordered`: exposed names match
  response names regardless of case and retain transport order.
- `cors_exposure_ignores_malformed_expose_values`: malformed expose-header
  values do not expose arbitrary headers.
- `forbidden_response_headers_are_hidden_before_public_response_creation`:
  prove that cookie headers are absent from the public list, not merely hidden
  by `get`.
- `internal_response_headers_remain_available_to_services`: trusted service
  hooks still receive raw `Set-Cookie` and other internal headers.
- `automatic_content_type_does_not_overwrite_explicit_header`: body metadata
  is inserted only when the caller did not supply `Content-Type`.
- `internal_origin_referer_and_cookie_headers_are_not_caller_headers`: these
  headers are added only in the internal transport path.

### Implementation approach

Use table-driven tests for forbidden names, safelisted media types, and range
forms. Keep one or two controller-level tests to verify that the unit-level
header behavior is preserved across request preparation and transport.

## 2. Request methods, bodies, and URL preparation

Primary files:

- `crates/net/src/api/request/body.rs`
- `crates/net/src/api/request/definition.rs`
- `crates/net/src/policy/request.rs`
- `crates/net/tests/requests.rs`

### Tests to add

- `standard_methods_are_normalized`: lowercase `delete`, `get`, `head`,
  `options`, `post`, and `put` become their canonical forms.
- `invalid_method_tokens_are_rejected`: malformed method bytes produce
  `InvalidMethod`.
- `forbidden_methods_are_rejected`: `CONNECT`, `TRACE`, and `TRACK` fail
  before network I/O.
- `no_cors_rejects_non_simple_methods`: `PUT`, `DELETE`, and custom methods
  fail in `NoCors` mode.
- `get_and_head_bodies_are_rejected`: include empty, replayable, one-shot,
  and streaming bodies.
- `body_constructors_report_metadata`: verify byte length and content type for
  bytes, text, URL-encoded, and custom-content-type bodies.
- `explicit_content_type_wins_over_body_metadata`: caller headers are not
  replaced by inferred metadata.
- `replayable_body_can_be_sent_multiple_times`: repeated requests reuse byte
  bodies successfully.
- `one_shot_bytes_fail_on_second_send`: cloned requests share consumption and
  the second attempt returns `BodyAlreadyConsumed`.
- `one_shot_stream_fails_on_second_send`: stream ownership is consumed by the
  first transport attempt.
- `one_shot_stream_propagates_source_error`: source I/O errors are exposed as
  transport/body errors and do not become successful cache entries.
- `stream_body_has_unknown_length`: streaming bodies report no known length.
- `keepalive_accepts_exact_limit`: a body exactly at the configured limit is
  accepted by size validation.
- `keepalive_rejects_body_over_limit`: one byte over the limit fails before
  transport.
- `keepalive_rejects_unknown_length`: streaming keepalive bodies fail the
  current validation boundary.
- `relative_url_resolves_against_environment_base`: verify path, query, and
  fragment resolution.
- `relative_url_without_base_is_rejected`: invalid URL failure occurs before
  scheduler admission.
- `url_credentials_are_rejected`: usernames and passwords never reach the
  transport.
- `origin_comparison_distinguishes_scheme_host_and_port`: default ports and
  nondefault ports are handled correctly.
- `referrer_policy_matrix_matches_expected_values`: cover same-origin,
  cross-origin, HTTPS-to-HTTP downgrade, `NoReferrer`, `Origin`, and
  `UnsafeUrl` cases.

### Implementation approach

Keep method/body/URL tests mostly as pure unit tests. Add a server-backed test
for each body category to verify the bytes and generated headers actually
sent by `ReqwestTransport`.

## 3. CORS, response types, and filtering

Primary files:

- `crates/net/src/policy/cors.rs`
- `crates/net/src/api/response/view.rs`
- `crates/net/tests/requests.rs`

### Tests to add

- `same_origin_response_is_basic`: readable status, URL, headers, and body
  are preserved.
- `same_origin_mode_rejects_cross_origin_response`: failure occurs before
  public exposure.
- `cors_requires_matching_allow_origin`: missing and mismatched values fail.
- `cors_wildcard_is_allowed_without_credentials`: `*` produces a readable
  CORS response for noncredentialed requests.
- `cors_wildcard_is_rejected_with_credentials`: credentialed requests require
  an exact origin.
- `credentialed_cors_requires_exact_true`: missing, mixed-case, and other
  values for `Access-Control-Allow-Credentials` are rejected according to the
  chosen contract.
- `no_cors_response_is_opaque`: status is zero, URL and headers are empty,
  cache provenance is hidden, and the body yields no bytes.
- `opaque_response_releases_or_drops_body_per_contract`: verify the scheduler
  permit and transport body are not leaked after filtering.
- `cors_exposes_safelisted_headers_only_by_default`: nonexposed headers stay
  hidden.
- `cors_exposes_explicit_headers_only`: `Access-Control-Expose-Headers`
  controls additional readable headers.
- `response_policy_failure_discards_internal_body`: service rejection cannot
  leave a readable body or cache entry.
- `error_response_is_not_exposed_as_transport_data`: typed request errors do
  not leak internal status, headers, or body.

### Unsupported-preflight tests

Until preflight is implemented, retain regression tests proving that the
following fail before server contact:

- cross-origin `PUT`, `DELETE`, and other non-simple methods;
- cross-origin requests with non-safelisted request headers;
- cross-origin requests with unsupported content types.

When preflight is implemented, replace these with tests for the `OPTIONS`
request, allowed methods and headers, credentials, preflight caching, failure,
and invalidation.

## 4. Null-body responses

Primary files:

- `crates/net/src/transport/reqwest.rs`
- `crates/net/src/api/response/mod.rs`
- `crates/net/tests/requests.rs`
- `crates/net/tests/streaming.rs`

### Tests to add

For each status below, test both `fetch` and `fetch_stream` with deliberately
present transport bytes and assert that the public body is empty:

- `HEAD` response;
- `101 Switching Protocols`;
- `204 No Content`;
- `205 Reset Content`;
- `304 Not Modified`.

Also verify that:

- null responses can be cached only according to the documented policy;
- null responses do not retain a scheduler permit unnecessarily;
- a malformed response that claims a null status but contains body bytes does
  not expose those bytes;
- buffered and streaming paths expose identical status, URL, headers, and
  body semantics.

## 5. Redirects and redirect-mode boundaries

Primary files:

- `crates/net/tests/redirects.rs`
- `crates/net/src/engine/execute.rs`
- `crates/net/src/api/request/context.rs`

### Tests for the current implementation

- `each_supported_redirect_status_is_rejected`: `301`, `302`, `303`, `307`,
  and `308` fail with `RedirectFailure`.
- `redirect_rejection_happens_before_public_headers`: no redirect response
  headers or body reach callers.
- `malformed_location_does_not_trigger_a_second_request`.
- `relative_location_is_not_followed_by_transport`.
- `cross_origin_location_is_not_followed_implicitly`.
- `manual_and_error_modes_fail_at_the_documented_boundary`: currently both
  modes are unsupported before network I/O.

### Tests when redirect following is implemented

- valid relative and absolute `Location` resolution;
- non-HTTP target rejection;
- 20-hop limit and loop detection;
- URL-list and redirect-count bookkeeping;
- `301`/`302`/`303` method and body rewriting;
- `307`/`308` method and body preservation;
- one-shot body replay failure;
- sensitive-header and credential stripping across origins;
- per-hop origin, CORS, cookie, security, integrity, and service checks;
- manual-mode opaque redirect responses;
- error-mode network errors.

## 6. Credentials and browser service hooks

Primary files:

- `crates/net/src/api/services.rs`
- `crates/net/src/engine/execute.rs`
- `crates/net/tests/requests.rs`

Create a recording `FetchServices` implementation and add tests for:

- `omit_does_not_request_or_store_cookies`;
- `same_origin_credentials_are_used_only_for_same_origin_targets`;
- `include_requires_browser_services`;
- `include_requests_cookie_header_before_transport`;
- `set_cookie_is_processed_only_after_response_approval`;
- `set_cookie_is_not_exposed_to_public_callers`;
- `cookie_service_errors_abort_the_request`;
- `request_policy_errors_precede_cookie_lookup`;
- `response_policy_errors_prevent_cookie_storage`;
- `service_worker_intercept_fails_closed_before_transport`;
- `network_service_worker_decision_reaches_transport`;
- `cache_partition_is_requested_only_for_cacheable_requests`;
- `different_partitions_do_not_share_cached_responses`;
- `response_header_observer_receives_internal_metadata`.

Assert call order, not only return values. In particular, a failed request
must not call cookie selection, a rejected response must not store cookies,
and a public response must never contain `Set-Cookie`.

## 7. Cache behavior and lifecycle

Primary files:

- `crates/net/src/cache/memory.rs`
- `crates/net/tests/cache.rs`
- `crates/net/tests/streaming.rs`

### Tests for the current cache subset

- `cache_control_max_age_zero_is_immediately_stale`;
- `cache_control_rejects_negative_malformed_and_overflowing_max_age`;
- `cache_control_handles_multiple_headers`;
- `no_store_and_no_cache_are_not_cached`;
- `force_cache_returns_stale_entry`;
- `default_does_not_return_stale_entry`;
- `reload_bypasses_existing_entry_and_replaces_it`;
- `only_if_cached_never_contacts_the_server`;
- `get_and_head_use_distinct_cache_keys`;
- `query_strings_remain_distinct_while_fragments_are_ignored`;
- `cache_partitions_are_isolated`;
- `cached_body_is_independent_for_each_caller`;
- `partial_body_consumption_does_not_insert_an_entry`;
- `body_drop_does_not_insert_an_entry`;
- `body_error_does_not_insert_an_entry`;
- `abort_does_not_insert_an_entry`;
- `policy_rejection_does_not_insert_an_entry`;
- `clearing_a_cache_shared_by_controller_clones_removes_all_entries`;
- `cached_response_is_rechecked_by_browser_services`.

### Tests for later HTTP-cache work

When standards-aware caching is implemented, add tests for:

- `Vary` matching and mismatch;
- request credentials and authorization affecting cache matching;
- `Date`, `Expires`, `Age`, and heuristic freshness;
- validators and conditional requests;
- `304` response merging;
- invalidation after unsafe methods;
- cache partition and response-tainting interactions;
- stale-while-revalidate and other supported directives;
- fresh public response views from a cloneable internal response.

## 8. Cancellation and scheduler lifecycle

Primary files:

- `crates/net/src/api/cancellation.rs`
- `crates/net/src/scheduling/scheduler.rs`
- `crates/net/src/api/response/body.rs`
- `crates/net/tests/scheduler.rs`
- `crates/net/tests/streaming.rs`

### Tests to add

- `aborted_signal_is_observed_by_all_clones`;
- `repeated_abort_is_idempotent`;
- `abort_before_fetch_skips_network_io`;
- `abort_while_waiting_for_scheduler_permit_skips_transport`;
- `abort_after_headers_ends_body_with_aborted_error`;
- `abort_during_body_streaming_does_not_cache`;
- `dropping_body_releases_scheduler_permit`;
- `body_error_releases_scheduler_permit`;
- `aborted_body_releases_scheduler_permit`;
- `canceled_request_does_not_reduce_future_scheduler_capacity`;
- `scheduler_limit_remains_enforced_after_cancellation`.

Use a blocking local server and a scheduler limit of one to make permit
ownership observable. Avoid sleeps where possible; use barriers or channels
to control when headers and chunks are released.

## 9. Transport robustness

Primary files:

- `crates/net/src/transport/reqwest.rs`
- `crates/net/tests/requests.rs`
- `crates/net/tests/streaming.rs`

Add controlled local-server cases for:

- valid chunked responses;
- truncated chunked responses;
- shorter and longer-than-declared `Content-Length` bodies;
- empty chunked responses;
- connection close before headers;
- connection close during the body;
- duplicate response headers;
- non-UTF-8 header values;
- large response chunks;
- configured user-agent and generated referer behavior;
- transport initialization failure, where configuration allows it to be
  induced deterministically.

### Response decompression

The current Reqwest transport enables gzip, Brotli, deflate, and zstd
decompression. Add one integration test per encoding:

- `gzip_response_is_decoded_before_public_body_exposure`;
- `brotli_response_is_decoded_before_public_body_exposure`;
- `deflate_response_is_decoded_before_public_body_exposure`;
- `zstd_response_is_decoded_before_public_body_exposure`.

For each encoding, verify both `fetch` and `fetch_stream`, decoded body bytes,
stream chunk behavior, and the public treatment of `Content-Encoding` and
`Content-Length`. Reqwest removes those headers after automatic decoding; the
test should lock down whether that matches the engine contract.

Also add tests for:

- `explicit_accept_encoding_is_preserved`: caller-provided encoding headers
  are not replaced by transport defaults;
- `range_requests_do_not_enable_automatic_encoding`: a request with `Range`
  does not receive an implicit `Accept-Encoding` header or unexpected body
  transformation;
- `generated_accept_encoding_is_internal`: automatic transport negotiation is
  not visible as a caller-controlled request header;
- `malformed_compressed_body_returns_transport_error`;
- `truncated_compressed_body_returns_transport_error`;
- `decompression_failure_does_not_enter_cache`;
- `abort_during_decompression_releases_permit`;
- `decoded_response_size_limit_rejects_expansion`;
- `decompression_ratio_limit_rejects_bomb_like_payload`;
- `decoded_body_cache_hits_are_independent`: each cache hit receives a fresh
  decoded body and public response view;
- `cache_policy_preserves_required_encoding_metadata`: cache matching retains
  any internal `Vary: Accept-Encoding`, validator, or wire metadata needed by
  the selected cache design.

The decompression tests should use deterministic compressed fixtures rather
than relying on a third-party server. Include small payloads, empty payloads,
multi-chunk payloads, and payloads whose decoded size is much larger than the
wire size.

Each failure test should assert both the error type and side effects: whether
the body was partially delivered, whether the scheduler permit was released,
whether cookies were stored, and whether the response entered the cache.

## 10. Unsupported feature contract tests

Keep a focused table-driven suite for features intentionally unsupported in
the current slice:

- non-HTTP schemes;
- navigation fetches;
- integrity metadata;
- keepalive execution;
- nondefault priority scheduling;
- `NoCache` revalidation;
- manual and error redirect modes;
- CORS preflight;
- service-worker interception without worker-response integration;
- browser-context requests without `FetchServices`.

For every case, assert that the error is `UnsupportedFeature` with the
documented feature name and that the test server received no request.

## Implementation sequence

Implement the tests in vertical slices so each group protects a complete
behavior boundary:

1. Add pure header, method, body, URL, origin, and referrer tests.
2. Add null-body tests to both buffered and streaming controller paths.
3. Add recording service-hook tests for credentials, cookies, policy, and
   cache partitioning.
4. Add cancellation and scheduler tests using deterministic barriers.
5. Complete cache lifecycle and cache-control edge cases.
6. Expand CORS exposure and failure tests, retaining preflight unsupported
   tests until preflight exists.
7. Add redirect boundary tests, then replace them incrementally with redirect
   conformance tests as controller-owned redirects are implemented.
8. Add transport framing, connection-failure, and response-decompression
   tests.
9. Select a Fetch subset from Web Platform Tests after the engine API and
   shared body model stabilize. Track intentionally unsupported tests
   explicitly.

## Verification and completion criteria

For each slice:

```text
cargo fmt --all
cargo test -p net
cargo clippy -p net --all-targets -- -D warnings
```

The test plan is complete for the current Fetch slice when every supported
public behavior has a success, boundary, and failure-timing test; every
intentional limitation has a no-network regression test; and buffered and
streaming paths agree on status, URL, headers, response type, null-body
behavior, cancellation, and cache effects.

The standards-oriented gaps remain separate work: full body consumption and
cloning, preflight, controller-owned redirects, credential/cookie policy,
standards-aware HTTP caching, integrity, navigation, service-worker response
integration, and browser policy providers.
