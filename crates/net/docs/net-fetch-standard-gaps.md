# Net crate: gaps against the Fetch Standard

Status: audit of the implementation as of 2026-10-09, including the current working-tree changes in `crates/net`.

This report compares `crates/net` with the [WHATWG Fetch Living Standard](https://fetch.spec.whatwg.org/), last updated 2026-10-06. It evaluates the networking crate as a browser-facing Fetch implementation, not merely as an HTTP client. A field or enum is counted as implemented only when it changes observable behavior at the appropriate policy boundary.

The crate currently provides an HTTP(S) transport with a Fetch-shaped request model: environment-based relative URL resolution, structured HTTP(S) origins, guarded ordered request headers, replayable and one-shot request bodies, automatic body metadata and `Origin`/`Referer` preparation, streamed response bodies, abort signaling, limited redirects through Reqwest, connection pooling, keepalive body-size validation, and a small in-memory cache. It is not yet Fetch-compatible for security-sensitive cross-origin behavior or for the full request/response algorithms.

## Summary

| Area                        | Current state                                                                                                                       | Gap severity |
| --------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- | ------------ |
| URL and scheme fetch        | HTTP(S) URLs and environment-relative URLs work; non-HTTP schemes do not                                                            | High         |
| Request headers and methods | Method validation and request/no-CORS guards exist; response filtering is absent                                                    | High         |
| Request and response bodies | Replayable bytes plus one-shot bytes/streams and basic metadata; Fetch Body semantics are absent                                    | Medium       |
| Credentials and cookies     | `CredentialsMode` exists, but the cookie store is stateless and never attaches or stores cookies                                    | Critical     |
| CORS and response tainting  | Checker accepts every response; no preflight or filtered responses                                                                  | Critical     |
| Redirects                   | Reqwest follows redirects independently of `redirect_mode`                                                                          | High         |
| HTTP cache                  | TTL-only in-memory cache; no HTTP cache matching/revalidation/partitioning                                                          | High         |
| Fetch policy integrations   | Keepalive size validation exists; CSP, mixed content, CORP, SRI, service workers, Fetch Metadata, and origin enforcement are absent | Critical     |
| Cancellation and lifecycle  | Basic abort signal works, but Fetch controller state/reasons/timing are absent                                                      | Medium       |
| Fetch API surface           | No Fetch `Headers`, `Request`, `Response`, or Body mixin semantics                                                                  | High         |
| Scheduling and transport    | HTTP/2, compression, pooling, and priority mapping are available; scheduler ordering and HTTP/3 are absent                          | Low/Medium   |

## Detailed gaps

### 1. URL parsing and scheme fetch

**Evidence:** [`RequestPolicy::validate_request`](../crates/net/src/policy/mod.rs) accepts only `http` and `https`, but resolves parseable relative references through [`FetchEnvironment::base_url`](../crates/net/src/request.rs). [`RequestController::fetch_stream`](../crates/net/src/controller.rs) initializes the request URL list and current URL before transport.

The standard defines fetch schemes as `about`, `blob`, `data`, `file`, and HTTP(S), with scheme-specific fetch behavior. The crate still has no local-URL, `data:` URL, `blob:` URL, or `file:` fetch implementation. Relative URLs now can be resolved against an optional environment/base URL, and requests record an initial/current URL and redirect count, but redirects do not yet update that state through a controller-owned Fetch loop. Origin state supports HTTP(S) tuple and opaque origins, but the complete local-URL, credentials, and opaque-origin bookkeeping is not implemented.

**Required work:** implement non-HTTP scheme fetches or explicitly scope this crate to HTTP(S) and reject unsupported modes at the public API boundary; track the URL list/current URL/redirect count through every redirect and complete the remaining origin bookkeeping.

### 2. Request methods and header guards

**Evidence:** [`Request::new` and `set_method`](../crates/net/src/request.rs) normalize and validate methods; [`HeaderList`](../crates/net/src/request.rs) has request, no-CORS request, response, immutable, and unrestricted guards; transport converts the permitted list in [`ReqwestTransport::send`](../crates/net/src/transport/reqwest.rs).

Fetch normalizes the standard methods, rejects forbidden methods such as `CONNECT`, `TRACE`, and `TRACK` at request construction, and applies request-header guards, forbidden request-header filtering, CORS-safelisted checks, and no-CORS restrictions. The implementation now covers these request-side checks and inserts the `Origin` header internally when required. It still does not provide the complete user-agent-owned header set or a separate cookie-header channel, and transport-specific details such as `Referer` are finalized internally.

The ordered multimap shape and mutation guards are directionally correct, but response headers are returned as a raw `reqwest::HeaderMap`, so Fetch's filtered/exposed-header semantics and special `Set-Cookie` handling are not represented.

**Required work:** extend the guarded header list with automatic user-agent-controlled headers, response header filtering, and a separate internal cookie-header channel.

### 3. Request bodies and body lifecycle

**Evidence:** [`RequestBody`](../crates/net/src/request.rs) supports replayable bytes, text, URL-encoded bytes, one-shot bytes, and one-shot streams, with known-length and content-type metadata. [`ResponseBody`](../crates/net/src/response.rs) remains a Rust stream, not a Fetch Body mixin.

The Fetch Standard's `BodyInit`/body algorithms support strings, URL-encoded data, `FormData`, `Blob`, `ArrayBuffer`/typed arrays, and streams. The crate now has streaming request input, text and URL-encoded constructors, automatic content-type metadata for supported constructors, known-length tracking, one-shot consumption errors, and rejection of bodies on `GET`/`HEAD`. It still lacks `FormData`, `Blob`, typed-array conversion, `duplex`, body cloning/`bodyUsed`, standardized body consumption/error state, keepalive aggregate quotas, and complete response null-body handling. It also does not yet model the rule that `HEAD`/`CONNECT` responses have no body.

**Required work:** extend the body abstraction with the remaining Fetch body types, aggregate keepalive quotas, and Fetch-compatible body consumption and cloning semantics.

### 4. Credentials, cookies, and authentication

**Evidence:** [`CookieStore::attach`](../crates/net/src/cookies/jar.rs) and [`process_response`](../crates/net/src/cookies/jar.rs) are no-ops. [`FetchContext.credentials`](../crates/net/src/request.rs) is stored and exposed, but is not consulted by the controller or transport. `FetchEnvironment` now also carries an optional `NetworkPartitionKey`, but no subsystem uses it yet.

Fetch credentials include cookies, TLS client certificates, and HTTP authentication entries. The crate has no cookie jar, domain/path matching, expiry, Secure/HttpOnly/SameSite enforcement, public-suffix checks, partitioning, response `Set-Cookie` processing, client certificate selection, or HTTP authentication entry handling. `omit`, `same-origin`, and `include` therefore produce the same behavior.

This is a critical security gap: once a cookie store is added, it must be keyed by the request's network partition/origin context and must be integrated with CORS credentials checks rather than simply attaching every matching cookie.

### 5. CORS, no-CORS, and response filtering

**Evidence:** [`CorsChecker::validate`](../crates/net/src/cors/validator.rs) returns `Ok(())` for every request and response. The request layer now rejects non-safelisted no-CORS methods and header values, but the controller still calls the permissive checker only after transport headers arrive and returns unfiltered response headers/body.

The implementation lacks the core CORS algorithm: although origin serialization, the `Origin` request header, and some no-CORS method/header checks now exist, there are no preflight `OPTIONS` requests, preflight cache, `Access-Control-Allow-*` validation, credentials interaction, redirect checks, or network-error conversion on failure. `RequestMode::SameOrigin` and `RequestMode::NoCors` remain largely inert at the controller boundary. No-CORS requests should use opaque filtered responses and only follow redirects in the allowed cases; the crate exposes status, URL, headers, and body directly.

It also lacks CORS-exposed header filtering and the distinction between internal, basic, CORS, opaque, and opaque-redirect filtered responses. The current raw `HeaderMap` can expose headers that Fetch deliberately hides.

**Required work:** implement request tainting and the CORS-preflight/CORS-check algorithms before exposing response data, with focused tests for simple, preflighted, credentialed, failed, no-CORS, and redirecting cross-origin requests.

### 6. Redirect handling

**Evidence:** [`Config.max_redirects`](../crates/net/src/config.rs) configures Reqwest's policy, while [`ReqwestTransport::new`](../crates/net/src/transport/reqwest.rs) enables it unconditionally. `Request` now stores `redirect_mode`, `url_list`, `current_url`, and `redirect_count`, but the controller and transport do not use the mode or record each Reqwest hop.

The standard's HTTP-redirect fetch algorithm validates the `Location` URL, rejects non-HTTP(S) redirect targets, caps the redirect count at 20, updates the request URL list, applies method/body/header changes for status codes, strips credentials and sensitive headers when required, and re-runs policy checks for each hop. `error` and `manual` modes have distinct observable results. The crate still delegates to Reqwest's follow policy, defaults to a configurable limit of 10, does not expose an intermediate/manual redirect response, and does not apply per-hop Fetch policy. Replayable versus one-shot request bodies are represented, but no controller-level redirect algorithm uses that distinction yet.

**Required work:** disable transport-owned automatic redirects and implement a controller-level redirect loop that honors `RedirectMode`, tracks every URL, replays or rejects bodies correctly, and invokes CORS/security checks on each hop.

### 7. HTTP cache and cache modes

**Evidence:** [`ResponseCache::insert`](../crates/net/src/cache/mod.rs) stores only successful responses with a parsed `Cache-Control: max-age`; [`cache_key`](../crates/net/src/cache/mod.rs) includes method and normalized URL while ignoring fragments; [`fetch_stream`](../crates/net/src/controller.rs) treats cache modes as local lookup switches.

The Fetch HTTP-network-or-cache algorithm relies on an HTTP cache, cache partitioning, freshness, validators, and revalidation. The current cache is process-local and keyed by method plus normalized URL, with fragment removal and a TTL derived from `max-age`. It does not vary by request headers or `Vary`, credentials, origin/network partition key, authorization, response tainting, or cache policy. It does not process `Date`, `Expires`, `Age`, validators, `Vary`, invalidation, 304 responses, or heuristic freshness.

`NoCache` bypasses the cache rather than revalidating it. `Reload` bypasses the cache but does not add the required revalidation/request directives. `Default` has no stale-while-revalidate behavior. `ForceCache` returns any stored entry but there is no standards-compliant stored-response match. `OnlyIfCached` does not enforce the same-origin restriction and returns a crate-specific `CacheMiss` rather than a Fetch network error. Cache entries are also buffered only after a caller fully consumes the body, which is a product choice but not the standard cache algorithm.

**Required work:** decide whether this crate owns a real HTTP cache or delegates to one, then implement cache matching, partitioning, freshness, conditional requests, 304 merging, invalidation, and each cache-mode algorithm.

### 8. Referrer, origin, and request context

**Evidence:** [`Referrer::Client`](../crates/net/src/request.rs) now resolves through `FetchEnvironment.referrer`, and [`referrer_value`](../crates/net/src/request.rs) applies the configured reduction policy. [`Origin`](../crates/net/src/request.rs) is structured and can parse/serialize HTTP(S) tuple origins or represent opaque origins; `apply_fetch_headers` derives an internal `Origin` header when required.

The referrer-policy enum and explicit/client referrer reduction logic are now present, and same-origin comparison is available for structured origins. The crate still does not derive same-site relationships, apply origin changes across redirects, or connect origin state to strict same-origin, credentials, CORS, and cookie behavior. Opaque origins are represented but are not integrated into those algorithms.

**Required work:** connect the structured origin and client environment to same-origin/same-site, credentials, CORS, cookie, and redirect policy, then centralize the remaining origin/referrer derivation before request headers are finalized.

### 9. Security and browser integration hooks

**Evidence:** [`RequestPolicy::validate_response`](../crates/net/src/policy/mod.rs#L26-L35) accepts every response. The request fields for destination, service workers, initiator, integrity, and keepalive are declared in [`Request`](../crates/net/src/request.rs#L297-L313), but no controller path consumes them. The crate itself documents missing persistent cookies, strict CORS, and HTTP/3 in [`lib.rs`](../crates/net/src/lib.rs#L20-L20).

The Fetch Standard coordinates with other web-platform policies. Missing or inert integrations include:

- Content Security Policy and mixed-content checks;
- Upgrade Insecure Requests;
- service-worker interception and bypass behavior;
- Fetch Metadata request headers and navigation/client context;
- Cross-Origin-Resource-Policy and MIME/nosniff blocking;
- Subresource Integrity verification of the response body;
- `keepalive` lifetime/quota enforcement and deferred fetch;
- network partition keys and HTTP cache partitions;
- offline/network-state integration;
- TLS client certificates and HTTP authentication entries;
- timing information for Resource Timing/Navigation Timing and server timing;
- early hints and response lifecycle callbacks.

Keepalive is no longer wholly inert: `Config.max_keepalive_body_size` and request validation reject known bodies over the configured 64 KiB default, while unknown-length keepalive bodies are rejected. The other security-sensitive fields, including `integrity`, `service_workers`, and request context, remain public state without enforcement and should either be wired to policy modules or documented as intentionally unsupported at the public API boundary.

### 10. Cancellation and fetch lifecycle

**Evidence:** [`AbortSignal`](../crates/net/src/cancellation.rs#L10-L66) is an atomic boolean plus notification, and transport/response streaming maps it to `RequestError::Aborted` in [`ReqwestTransport::send`](../crates/net/src/transport/reqwest.rs#L89-L92) and [`ResponseBody::from_stream_with_signal`](../crates/net/src/response.rs#L79-L100).

Basic cancellation is implemented, but Fetch controllers distinguish ongoing, terminated, and aborted states, preserve a serialized abort reason, cancel all fetch stages, and coordinate body/error handover. The crate has no abort reason, no distinction between termination and abort, no lifecycle callbacks, and no timing/reporting hooks. Cancellation during a redirect, cache revalidation, cookie processing, or body capture is not modeled as a Fetch algorithm state transition.

### 11. Response representation and Fetch API behavior

**Evidence:** [`Response`](../crates/net/src/response.rs#L129-L140) and [`StreamingResponse`](../crates/net/src/response.rs#L143-L156) expose raw status, URL, headers, and bytes. The crate exports Rust structs and a `Stream`, not the Fetch `Headers`, `Request`, `Response`, and Body interfaces.

Missing response semantics include response type, URL list/URL exposure rules, redirected flag, status/message distinctions, null/filtered responses, header guards, body consumption helpers (`arrayBuffer`, `blob`, `bytes`, `formData`, `json`, `text`), body locking/disturbance, cloning, and standardized network errors. This may be acceptable if `crates/net` is deliberately a lower-level internal boundary, but it is a gap relative to the Fetch API and should be stated in the crate contract.

### 12. Scheduling and transport coverage

**Evidence:** [`Request::transport_priority`](../crates/net/src/request.rs) maps Fetch low/auto/high priority to scheduler priority classes, but [`RequestScheduler::submit`](../crates/net/src/scheduler.rs) still receives `_priority` and never reorders admission. Reqwest is configured for Rustls, HTTP/2, compression, pooling, and a custom user agent in [`ReqwestTransport::new`](../crates/net/src/transport/reqwest.rs).

The semaphore provides concurrency admission, and Fetch priority now reaches the scheduler boundary, but it still has no effect on ordering. There are no per-origin/host limits, network partition-aware connection pools, connection timing records, proxy policy, offline mode, or HTTP/3 transport. These are not all required for a minimal HTTP(S) Fetch subset, but they are gaps from the browser-wide behavior described by the standard and the crate's stated browser boundary.

## Recommended implementation order

1. Establish the contract: either implement a browser-facing Fetch subset or rename/document this as an HTTP(S) transport API. Mark currently inert public fields as unsupported until they work.
2. Build structured environment/origin/request state and enforce header/method guards before transport.
3. Implement credentials/cookie policy and strict same-origin/CORS/no-CORS response filtering before exposing response data.
4. Move redirects into the controller and implement Fetch redirect modes, URL-list tracking, and per-hop policy.
5. Replace the TTL map with a partitioned HTTP cache or integrate an existing standards-aware cache.
6. Add body, integrity, keepalive, service-worker, CSP/mixed-content, MIME/CORP, and timing hooks as the surrounding browser subsystems become available.

## Suggested regression-test matrix

At minimum, add tests for:

- forbidden methods/headers and no-CORS header filtering;
- same-origin, cross-origin simple CORS, failed CORS, credentialed CORS, and preflight caching;
- opaque and opaque-redirect responses;
- cookie `Set-Cookie` persistence, matching, expiry, SameSite, and credentials modes;
- redirect `follow`, `error`, and `manual`, including method/body rewriting and 20-hop limits;
- cache `default`, `no-store`, `reload`, `no-cache`, `force-cache`, and `only-if-cached`, including validators and `Vary`;
- `HEAD`, null-body statuses, body errors, cancellation reasons, SRI success/failure, and keepalive quotas;
- `data:`, `blob:`, `about:`, and unsupported-scheme behavior.
