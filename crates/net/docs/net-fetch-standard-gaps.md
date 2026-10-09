# Net crate: gaps against the Fetch Standard

Step 1 of the Fetch engine plan now separates complete internal responses from
filtered public views. Public responses use an owned status code and guarded
header list, hide cookie headers, apply basic/simple-CORS/opaque exposure, and
enforce null bodies. The detailed audit below records the wider unfinished
Fetch algorithms; see `fetch-engine-contract.md` for the current supported
subset and capability errors.

Status: audit of the implementation as of 2026-10-09, including the current working-tree changes in `crates/net`.

This report compares `crates/net` with the [WHATWG Fetch Living Standard](https://fetch.spec.whatwg.org/), last updated 2026-10-06. It evaluates the networking crate as a browser-facing Fetch implementation, not merely as an HTTP client. A field or enum is counted as implemented only when it changes observable behavior at the appropriate policy boundary.

The crate currently provides an HTTP(S) transport with a Fetch-shaped request model: environment-based relative URL resolution, structured HTTP(S) origins, guarded ordered request headers, replayable and one-shot request bodies, automatic body metadata and `Origin`/`Referer` preparation, filtered streamed responses, abort signaling, connection pooling, keepalive body-size validation, and a small in-memory cache. Redirects fail closed until the controller owns the redirect loop. Browser-context requests require a `FetchServices` provider. The crate is not yet Fetch-compatible for the full request/response algorithms.

## Summary

| Area                        | Current state                                                                                                                            | Gap severity |
| --------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- | ------------ |
| URL and scheme fetch        | HTTP(S) URLs and environment-relative URLs work; non-HTTP schemes do not                                                                 | High         |
| Request headers and methods | Method validation and request/no-CORS guards exist; response headers are filtered before exposure                                        | High         |
| Request and response bodies | Replayable bytes plus one-shot bytes/streams and basic metadata; Fetch Body semantics are absent                                         | Medium       |
| Credentials and cookies     | `FetchServices` can supply and store cookies; matching, persistence, and policy belong to its provider                                   | Critical     |
| CORS and response tainting  | Simple CORS checks and basic/opaque views exist; preflight and redirect checks are absent                                                | Critical     |
| Redirects                   | Transport following is disabled; redirect statuses fail before public exposure until step 7                                              | High         |
| HTTP cache                  | TTL-only in-memory cache with provider-supplied partition keys; full matching/revalidation is absent                                     | High         |
| Fetch policy integrations   | Browser-context requests require `FetchServices`; CSP, mixed content, CORP, SRI, and worker interception remain provider/later-step work | Critical     |
| Cancellation and lifecycle  | Basic abort signal works, but Fetch controller state/reasons/timing are absent                                                           | Medium       |
| Fetch API surface           | Rust response types and guarded headers exist; shared Fetch Body and constructor semantics are absent                                    | High         |
| Scheduling and transport    | HTTP/2, compression, pooling, and priority mapping are available; scheduler ordering and HTTP/3 are absent                               | Low/Medium   |

## Detailed gaps

### 1. URL parsing and scheme fetch

**Evidence:** [`RequestPolicy::validate_request`](../crates/net/src/policy/mod.rs) accepts only `http` and `https`, but resolves parseable relative references through [`FetchEnvironment::base_url`](../crates/net/src/request.rs). [`RequestController::fetch_stream`](../crates/net/src/controller.rs) initializes the request URL list and current URL before transport.

The standard defines fetch schemes as `about`, `blob`, `data`, `file`, and HTTP(S), with scheme-specific fetch behavior. The crate still has no local-URL, `data:` URL, `blob:` URL, or `file:` fetch implementation. Relative URLs now can be resolved against an optional environment/base URL, and requests record an initial/current URL and redirect count, but redirects do not yet update that state through a controller-owned Fetch loop. Origin state supports HTTP(S) tuple and opaque origins, but the complete local-URL, credentials, and opaque-origin bookkeeping is not implemented.

**Required work:** implement non-HTTP scheme fetches or explicitly scope this crate to HTTP(S) and reject unsupported modes at the public API boundary; track the URL list/current URL/redirect count through every redirect and complete the remaining origin bookkeeping.

### 2. Request methods and header guards

**Evidence:** [`Request::new` and `set_method`](../crates/net/src/request.rs) normalize and validate methods; [`HeaderList`](../crates/net/src/request.rs) has request, no-CORS request, response, immutable, and unrestricted guards; transport converts the permitted list in [`ReqwestTransport::send`](../crates/net/src/transport/reqwest.rs).

Fetch normalizes the standard methods, rejects forbidden methods such as `CONNECT`, `TRACE`, and `TRACK` at request construction, and applies request-header guards, forbidden request-header filtering, CORS-safelisted checks, and no-CORS restrictions. The implementation now covers these request-side checks and inserts the `Origin` header internally when required. It still does not provide the complete user-agent-owned header set or a separate cookie-header channel, and transport-specific details such as `Referer` are finalized internally.

The ordered multimap shape and mutation guards now support immutable exposed response headers. The internal response retains `Set-Cookie`, while the public view removes it. Full Fetch `Headers` combination and iteration semantics remain unfinished.

**Required work:** extend the guarded header list with automatic user-agent-controlled headers, response header filtering, and a separate internal cookie-header channel.

### 3. Request bodies and body lifecycle

**Evidence:** [`RequestBody`](../crates/net/src/request.rs) supports replayable bytes, text, URL-encoded bytes, one-shot bytes, and one-shot streams, with known-length and content-type metadata. [`ResponseBody`](../crates/net/src/response.rs) remains a Rust stream, not a Fetch Body mixin.

The Fetch Standard's `BodyInit`/body algorithms support strings, URL-encoded data, `FormData`, `Blob`, `ArrayBuffer`/typed arrays, and streams. The crate now has streaming request input, text and URL-encoded constructors, automatic content-type metadata for supported constructors, known-length tracking, one-shot consumption errors, and rejection of bodies on `GET`/`HEAD`. It still lacks `FormData`, `Blob`, typed-array conversion, `duplex`, body cloning/`bodyUsed`, standardized body consumption/error state, keepalive aggregate quotas, and complete response null-body handling. It also does not yet model the rule that `HEAD`/`CONNECT` responses have no body.

**Required work:** extend the body abstraction with the remaining Fetch body types, aggregate keepalive quotas, and Fetch-compatible body consumption and cloning semantics.

### 4. Credentials, cookies, and authentication

**Evidence:** [`FetchServices`](../src/services.rs) supplies cookie selection and storage decisions for browser-context requests. The controller calls those methods only when the credentials mode permits credentials. The engine itself has no durable cookie store. `FetchEnvironment` carries an optional `NetworkPartitionKey`, while the provider supplies the process-local cache partition key.

Fetch credentials include cookies, TLS client certificates, and HTTP authentication entries. The engine delegates cookie matching, persistence, and policy to its provider; it does not implement those rules. TLS client certificate selection and HTTP authentication entries are still absent. `omit`, `same-origin`, and `include` now determine whether provider credential methods are invoked.

This is a critical security gap: once a cookie store is added, it must be keyed by the request's network partition/origin context and must be integrated with CORS credentials checks rather than simply attaching every matching cookie.

### 5. CORS, no-CORS, and response filtering

**Evidence:** [`CorsChecker::validate`](../crates/net/src/cors/validator.rs) enforces same-origin and simple CORS checks; [`InternalResponse::expose`](../crates/net/src/response.rs) constructs a filtered view before callers see headers or body. Preflight-required requests fail before transport.

The implementation still lacks preflight `OPTIONS`, a preflight cache, complete `Access-Control-Allow-*` parsing, cross-origin credentials, and per-hop redirect checks. `SameOrigin` fails closed on cross-origin results, and `NoCors` produces an opaque view, but the complete Fetch algorithms remain unfinished.

Basic, CORS, and opaque views now filter headers and metadata. Opaque-redirect construction awaits controller-owned redirects. CORS wildcard exposure and other full `Headers` rules remain unfinished.

**Required work:** implement request tainting and the CORS-preflight/CORS-check algorithms before exposing response data, with focused tests for simple, preflighted, credentialed, failed, no-CORS, and redirecting cross-origin requests.

### 6. Redirect handling

**Evidence:** [`ReqwestTransport::new`](../src/transport/reqwest.rs) disables automatic redirects. The controller returns `RedirectFailure` for Fetch redirect statuses before public exposure. `Request` and `InternalResponse` retain URL and redirect state for the one supported exchange; no redirect hop is followed.

The standard's HTTP-redirect fetch algorithm validates the `Location` URL, rejects non-HTTP(S) redirect targets, caps the redirect count at 20, updates the request URL list, applies method/body/header changes for status codes, strips credentials and sensitive headers when required, and re-runs policy checks for each hop. `error` and `manual` modes have distinct observable results. All of that remains for step 7; the current controller rejects redirects rather than delegating them to Reqwest.

**Required work:** implement a controller-level redirect loop that honors `RedirectMode`, tracks every URL, replays or rejects bodies correctly, and invokes CORS/security checks on each hop.

### 7. HTTP cache and cache modes

**Evidence:** [`ResponseCache::insert`](../crates/net/src/cache/mod.rs) stores only successful responses with a parsed `Cache-Control: max-age`; [`cache_key`](../crates/net/src/cache/mod.rs) includes method and normalized URL while ignoring fragments; [`fetch_stream`](../crates/net/src/controller.rs) treats cache modes as local lookup switches.

The Fetch HTTP-network-or-cache algorithm relies on an HTTP cache, cache partitioning, freshness, validators, and revalidation. The current cache is process-local and keyed by provider-supplied partition, method, and normalized URL, with fragment removal and a TTL derived from `max-age`. It does not vary by request headers or `Vary`, credentials, authorization, response tainting, or full cache policy. It does not process `Date`, `Expires`, `Age`, validators, `Vary`, invalidation, 304 responses, or heuristic freshness.

`NoCache` now fails with an explicit unsupported-feature error until revalidation exists. `Reload` bypasses the cache but does not add the required revalidation/request directives. `Default` has no stale-while-revalidate behavior. `ForceCache` returns any stored entry but there is no standards-compliant stored-response match. `OnlyIfCached` does not enforce the same-origin restriction; misses now return a Fetch network error. Cache entries are also buffered only after a caller fully consumes the body, which is a product choice but not the standard cache algorithm.

**Required work:** decide whether this crate owns a real HTTP cache or delegates to one, then implement cache matching, partitioning, freshness, conditional requests, 304 merging, invalidation, and each cache-mode algorithm.

### 8. Referrer, origin, and request context

**Evidence:** [`Referrer::Client`](../crates/net/src/request.rs) now resolves through `FetchEnvironment.referrer`, and [`referrer_value`](../crates/net/src/request.rs) applies the configured reduction policy. [`Origin`](../crates/net/src/request.rs) is structured and can parse/serialize HTTP(S) tuple origins or represent opaque origins; `apply_fetch_headers` derives an internal `Origin` header when required.

The referrer-policy enum and explicit/client referrer reduction logic are now present, and same-origin comparison is available for structured origins. The crate still does not derive same-site relationships, apply origin changes across redirects, or connect origin state to strict same-origin, credentials, CORS, and cookie behavior. Opaque origins are represented but are not integrated into those algorithms.

**Required work:** connect the structured origin and client environment to same-origin/same-site, credentials, CORS, cookie, and redirect policy, then centralize the remaining origin/referrer derivation before request headers are finalized.

### 9. Security and browser integration hooks

**Evidence:** Browser-context requests require [`FetchServices`](../src/services.rs) request and response decisions. Integrity, navigation fetch, and keepalive lifetime are rejected before transport. A provider may authorize destination/initiator context and return a service-worker network decision; an interception decision fails closed until worker responses are supported.

The Fetch Standard coordinates with other web-platform policies. Remaining integrations include:

- Content Security Policy and mixed-content checks;
- Upgrade Insecure Requests;
- service-worker interception and bypass behavior;
- Fetch Metadata request headers and navigation/client context;
- Cross-Origin-Resource-Policy and MIME/nosniff blocking;
- Subresource Integrity verification of the response body;
- `keepalive` lifetime/quota enforcement and deferred fetch;
- durable network partition state and standards-aware HTTP cache partitions;
- offline/network-state integration;
- TLS client certificates and HTTP authentication entries;
- timing information for Resource Timing/Navigation Timing and server timing;
- early hints and response lifecycle callbacks.

`Config.max_keepalive_body_size` still checks the known body length, but even an in-quota keepalive request fails with `UnsupportedFeature` until lifetime handling exists. Integrity and navigation fetch also fail explicitly. Browser policy, cookie, worker-network, cache-partition, and diagnostics inputs are supplied by the provider interface; their full browser-owned implementations remain outside this crate.

### 10. Cancellation and fetch lifecycle

**Evidence:** [`AbortSignal`](../crates/net/src/cancellation.rs#L10-L66) is an atomic boolean plus notification, and transport/response streaming maps it to `RequestError::Aborted` in [`ReqwestTransport::send`](../crates/net/src/transport/reqwest.rs#L89-L92) and [`ResponseBody::from_stream_with_signal`](../crates/net/src/response.rs#L79-L100).

Basic cancellation is implemented, but Fetch controllers distinguish ongoing, terminated, and aborted states, preserve a serialized abort reason, cancel all fetch stages, and coordinate body/error handover. The crate has no abort reason, no distinction between termination and abort, no lifecycle callbacks, and no timing/reporting hooks. Cancellation during a redirect, cache revalidation, cookie processing, or body capture is not modeled as a Fetch algorithm state transition.

### 11. Response representation and Fetch API behavior

**Evidence:** [`Response`](../src/response.rs) and [`StreamingResponse`](../src/response.rs) now expose filtered status, URL, headers, response type, and body. The private internal representation retains full transport metadata. The crate still exports Rust structs and a stream, not JavaScript Fetch interfaces.

Remaining response work includes full redirect URL-list accuracy, response constructors, body consumption helpers (`arrayBuffer`, `blob`, `bytes`, `formData`, `json`, `text`), body locking/disturbance, stream cloning, and complete network-error construction. The supported Rust subset and limitations are stated in `fetch-engine-contract.md`.

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
