# Net crate: gaps against the Fetch Standard

Steps 1 and 2 of the Fetch engine plan now separate complete internal
responses from filtered public views and complete the shared guarded header
model. Public responses use an owned status code and guarded header list, hide
cookie headers, apply basic/CORS/opaque exposure, and enforce the supported
null-body rules. The detailed audit below records the wider unfinished Fetch
algorithms; see `fetch-engine-contract.md` for the current supported subset and
capability errors.

Status: audit of the implementation as of 2026-10-15, including the current
committed `crates/net` implementation and the Feature 4.4 plan updates.

This report compares `crates/net` with the [WHATWG Fetch Living Standard](https://fetch.spec.whatwg.org/), last updated 2026-10-06. It evaluates the networking crate as a browser-facing Fetch implementation, not merely as an HTTP client. A field or enum is counted as implemented only when it changes observable behavior at the appropriate policy boundary.

The crate currently provides an HTTP(S) transport with a Fetch-shaped request model: environment-based relative URL resolution, structured HTTP(S) origins, guarded ordered request headers, replayable and one-shot request bodies, automatic body metadata and `Origin`/`Referer` preparation, filtered streamed responses, controller-owned redirect handling, abort signaling, connection pooling, keepalive body-size validation, and a small in-memory cache. Browser-context requests require a `FetchServices` provider. The crate is not yet Fetch-compatible for the full request/response algorithms. The dispatcher recognizes
`http`, `https`, `data`, `blob`, `file`, `byui`, and `about`. `data:` is
engine-owned and currently decodes percent-encoded and base64 payloads;
`blob:`, `file:`, and `byui:` resolve through installed browser-owned providers
and reach the shared response pipeline; the engine-owned `about:blank` case is
implemented while other `about:` URLs remain unsupported.

## Summary

| Area                        | Current state                                                                                                                                                 | Gap severity |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------ |
| URL and scheme fetch        | HTTP(S), relative URLs, `data:`, `blob:`, `file:`, `byui:`, and `about:blank` fetch work; other `about:` URLs are rejected                                    | High         |
| Request headers and methods | Method validation, request/no-CORS guards, internal generated-header channels, and response filtering exist; later redirect/header rules remain               | Medium       |
| Request and response bodies | Shared buffered/streaming `Body`, text/blob/form-data/JSON/array-buffer helpers, and null-body handling exist; cloning/teeing and full Fetch semantics remain | Medium       |
| Credentials and cookies     | `FetchServices` can supply and store cookies; matching, persistence, and policy belong to its provider                                                        | Critical     |
| CORS and response tainting  | Simple CORS checks and basic/opaque views exist; preflight and redirect checks are absent                                                                     | Critical     |
| Redirects                   | Controller follows validated HTTP(S) redirects with `follow`, `error`, and `manual` modes; full browser policy remains incomplete                             | High         |
| HTTP cache                  | TTL-only in-memory cache with provider-supplied partition keys; full matching/revalidation is absent                                                          | High         |
| Fetch policy integrations   | Browser-context requests require `FetchServices`; CSP, mixed content, CORP, SRI, and worker interception remain provider/later-step work                      | Critical     |
| Cancellation and lifecycle  | Basic abort signal works, but Fetch controller state/reasons/timing are absent                                                                                | Medium       |
| Fetch API surface           | Rust `Request`, filtered `Response`, guarded `Headers`, shared `Body`, and body helpers exist; JavaScript bindings, response constructors, and cloning remain | High         |
| Scheduling and transport    | HTTP/2, compression, pooling, and priority mapping are available; scheduler ordering and HTTP/3 are absent                                                    | Low/Medium   |

## Detailed gaps

### 1. URL parsing and scheme fetch

**Evidence:** [`RequestPolicy::validate_request`](../src/policy/request.rs)
resolves parseable relative references through
[`FetchEnvironment::base_url`](../src/api/request/context.rs) and classifies
`http`, `https`, `data`, `blob`, `file`, `byui`, and `about` through
`FetchScheme`. [`RequestController::fetch`](../src/engine/execute.rs) prepares
the URL and dispatches through [`fetch_scheme`](../src/engine/schemes/mod.rs).

Engine-owned `data:` fetching currently parses percent-encoded and base64
payloads, assigns a generated content type, creates a status-200 internal
response, and uses the shared response/body exposure path. Provider-neutral
resource results plus `BlobProvider`, `FileProvider`, and `PageProvider` are
public APIs in [`api/providers`](../src/api/providers/mod.rs), and
`SchemeProviders` is stored by the controller. The installed `blob:`, `file:`,
and `byui:` providers are invoked exactly once and their resources are
validated and finalized through the shared response path. Absent matching
providers return `MissingProvider`. The engine-owned `about:blank` case is
handled directly; every other `about:` URL returns `UnsupportedFeature`.

Relative URLs and HTTP(S) origins work, and opaque origins are representable.
The HTTP controller updates the URL list/current URL through its redirect loop;
provider-assigned origins and local-scheme URL-list, credential, body, and
filtering semantics are covered for the current provider-backed handlers;
browser composition and browser-owned provider edge cases remain.

**Required work:** finish the remaining `data:` edge cases; compose the
browser-owned providers; and track
URL-list/current-URL/redirect state and policy through every redirect. Provider
metadata and the shared finalization path are implemented. Provider resolution,
stream cancellation, failure, and body-drop cleanup are covered for the current
local handlers.

### 2. Request methods and header guards

**Evidence:** [`Request::new` and `set_method`](../src/api/request/definition.rs) normalize and validate methods; [`Headers`](../src/api/headers.rs) has request, no-CORS request, response, immutable, and unrestricted guards; transport converts the permitted list in [`ReqwestTransport::send`](../src/transport/reqwest.rs).

Fetch normalizes the standard methods, rejects forbidden methods such as `CONNECT`, `TRACE`, and `TRACK` at request construction, and applies request-header guards, forbidden request-header filtering, CORS-safelisted checks, and no-CORS restrictions. The implementation now covers these request-side checks and keeps generated `Origin`, body metadata, `Referer`, and cookie headers in an internal transport channel rather than mutating caller headers. It still does not provide the complete user-agent-owned header set or the redirect-time header rules.

The ordered multimap shape and mutation guards now support immutable exposed response headers. The internal response retains `Set-Cookie`, while the public view removes it. Duplicate combination, ordering, cookie separation, guard transitions, and the supported CORS exposure rules are implemented.

**Required work:** complete the remaining user-agent-owned header set and redirect-time header mutation rules. Header construction, mutation guards, duplicate combination, cookie separation, internal channels, and response filtering are no longer gaps in this audit.

### 3. Request bodies and body lifecycle

**Evidence:** [`Body`](../src/api/body/mod.rs) now backs request input plus buffered and streaming response bodies. It supports replayable bytes, text, URL-encoded bytes, blob-like bytes, one-shot bytes, and one-shot streams, with known-length, content-type, locking, and consumption metadata.

The Fetch Standard's `BodyInit`/body algorithms support strings, URL-encoded
data, `FormData`, `Blob`, `ArrayBuffer`/typed arrays, and streams. The crate
now has streaming request input, text and URL-encoded constructors, blob-like
bytes, `FormData` serialization/parsing for supported URL-encoded and
multipart forms, automatic content-type metadata, known-length tracking,
body-used errors, `bytes`, `array_buffer`, `text`, `blob`, `json`, and
`form_data` helpers, rejection of bodies on `GET`/`HEAD`, and null-body
handling for `HEAD` plus supported null-body status codes.

It still lacks Fetch-compatible stream teeing/cloning, a public response clone
operation, typed-array/JavaScript conversion (which belongs in `webapis`),
`duplex`, keepalive aggregate quotas, and the Fetch rule that a successful
`CONNECT` response has no body. The provider stream variant is defined, and the
blob, file, and page handlers exercise it; stream cancellation, failure, and
unread/partial body-drop cleanup are covered for that provider boundary.

**Required work:** finish body cloning/teeing and the remaining general
lifecycle semantics, define
the Rust-to-JavaScript body conversion boundary, add aggregate keepalive
quotas, and apply the body rules consistently to every scheme handler.

### 4. Credentials, cookies, and authentication

**Evidence:** [`FetchServices`](../src/api/services.rs) supplies cookie selection and storage decisions for browser-context requests. The controller calls those methods only when the credentials mode permits credentials. The engine itself has no durable cookie store. `FetchEnvironment` carries an optional `NetworkPartitionKey`, while the provider supplies the process-local cache partition key.

Fetch credentials include cookies, TLS client certificates, and HTTP authentication entries. The engine delegates cookie matching, persistence, and policy to its provider; it does not implement those rules. TLS client certificate selection and HTTP authentication entries are still absent. `omit`, `same-origin`, and `include` now determine whether provider credential methods are invoked.

This remains a critical integration gap: the provider-owned cookie store must be keyed by the request's network partition/origin context and integrated with CORS credentials checks rather than simply attaching every matching cookie.

### 5. CORS, no-CORS, and response filtering

**Evidence:** [`CorsChecker::response_type`](../src/policy/cors.rs) enforces same-origin and simple CORS checks; [`InternalResponse::expose`](../src/api/response/view.rs) constructs a filtered view before callers see headers or body. Preflight-required requests fail before transport.

The implementation still lacks preflight `OPTIONS`, a preflight cache, complete `Access-Control-Allow-*` parsing, and per-hop redirect checks. Basic credential checks for simple CORS responses exist, but credentialed preflight and redirect behavior remain absent. `SameOrigin` fails closed on cross-origin results, and `NoCors` produces an opaque view, but the complete Fetch algorithms remain unfinished.

Basic, CORS, and opaque views now filter headers and metadata. CORS exposure
parsing covers safelisted names, explicit lists, wildcard credentials rules,
duplicates, and malformed metadata. Opaque-redirect construction is now
present for manual HTTP redirects, and network-error response construction
remains outside the current response view. `data:` and `blob:` use the shared
finalization path, including provider-supplied origins, opaque-origin behavior,
cookie/cache exclusion, and response filtering. File and page local-scheme
response coverage remains unimplemented.

**Required work:** implement request tainting and the CORS-preflight/CORS-check
algorithms before exposing response data, extract one reusable response
finalization path for HTTP and local schemes, validate provider-assigned
origins fail-closed, and add focused tests for simple, preflighted,
credentialed, failed, no-CORS, local-origin, and redirecting cross-origin
requests.

### 6. Redirect handling

**Evidence:** [`ReqwestTransport::new`](../src/transport/reqwest.rs) disables automatic redirects, and [`RequestController::fetch_http`](../src/engine/schemes/http.rs) owns the redirect loop. The controller resolves and validates `Location`, caps redirects at 20, tracks the URL list/current URL/count, applies Fetch method/body changes, strips sensitive headers on origin changes, and supports `follow`, `error`, and opaque `manual` results. 307/308 redirects reject non-replayable request bodies.

The standard's complete redirect algorithm still includes more detailed per-hop CORS, credentials, referrer, cache, and security integration than this crate currently provides. Redirect targets are intended to remain limited to HTTP(S), but the redirect-specific validator still needs an explicit scheme guard because the shared target validator also serves local-scheme dispatch. Malformed or unsupported redirects otherwise fail closed with `RedirectFailure` or the relevant typed policy error.

**Required work:** complete per-hop CORS/security and credential policy, redirect-aware cache behavior, and the remaining browser integrations.

### 7. HTTP cache and cache modes

**Evidence:** [`ResponseCache::insert`](../src/cache/memory.rs) stores only successful responses with a parsed `Cache-Control: max-age`; [`cache_key`](../src/cache/memory.rs) includes method and normalized URL while ignoring fragments; [`fetch`](../src/engine/execute.rs) treats cache modes as local lookup switches.

The Fetch HTTP-network-or-cache algorithm relies on an HTTP cache, cache partitioning, freshness, validators, and revalidation. The current cache is process-local and keyed by provider-supplied partition, method, and normalized URL, with fragment removal and a TTL derived from `max-age`. It does not vary by request headers or `Vary`, credentials, authorization, response tainting, or full cache policy. It does not process `Date`, `Expires`, `Age`, validators, `Vary`, invalidation, 304 responses, or heuristic freshness.

`NoCache` now fails with an explicit unsupported-feature error until revalidation exists. `Reload` bypasses the cache but does not add the required revalidation/request directives. `Default` has no stale-while-revalidate behavior. `ForceCache` returns any stored entry but there is no standards-compliant stored-response match. `OnlyIfCached` does not enforce the same-origin restriction; misses now return a Fetch network error. Cache entries are also buffered only after a caller fully consumes the body, which is a product choice but not the standard cache algorithm.

**Required work:** decide whether this crate owns a real HTTP cache or delegates to one, then implement cache matching, partitioning, freshness, conditional requests, 304 merging, invalidation, and each cache-mode algorithm.

### 8. Referrer, origin, and request context

**Evidence:** [`Referrer::Client`](../src/api/request/definition.rs) now resolves through `FetchEnvironment.referrer`, and [`referrer_value`](../src/api/request/definition.rs) applies the configured reduction policy. [`Origin`](../src/api/request/origin.rs) is structured and can parse/serialize HTTP(S) tuple origins or represent opaque origins; [`Request::fetch_headers`](../src/api/request/definition.rs) derives an internal `Origin` header when required.

The referrer-policy enum and explicit/client referrer reduction logic are now present, and same-origin comparison is available for structured origins. The crate still does not derive same-site relationships, apply origin changes across redirects, or connect origin state to strict same-origin, credentials, CORS, and cookie behavior. Opaque origins are represented but are not integrated into those algorithms.

**Required work:** connect the structured origin and client environment to same-origin/same-site, credentials, CORS, cookie, and redirect policy, then centralize the remaining origin/referrer derivation before request headers are finalized.

### 9. Security and browser integration hooks

**Evidence:** Browser-context requests require [`FetchServices`](../src/api/services.rs) request and response decisions. Integrity, navigation fetch, and keepalive lifetime are rejected before transport. A provider may authorize destination/initiator context and return a service-worker network decision; an interception decision fails closed until worker responses are supported.

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

Feature 4.4 adds another explicit integration boundary. `net` owns only the
provider-neutral `SchemeResource`/`SchemeBody` representation and provider
traits. Browser/storage/security owners must provide blob registry and
revocation state, authorized file access and sandbox policy, and registered
`byui:` page lookup. The browser executable now installs its browser-owned
`byui:` provider through `RequestController::builder`; browser-wide
`FetchServices` composition and concrete blob/file providers remain outside
`net`. Provider errors, provider-assigned origins, metadata
validation, exact-once provider selection, and cancellation/lifetime behavior
are implemented for the blob, file, and page paths and covered by focused
scheme tests; browser-wide composition and concrete provider implementations
remain outside `net`.

`Config.max_keepalive_body_size` still checks the known body length, but even
an in-quota keepalive request fails with `UnsupportedFeature` until lifetime
handling exists. Integrity and navigation fetch also fail explicitly. Browser
policy, cookie, worker-network, cache-partition, diagnostics, and
scheme-provider inputs are supplied by browser-owned interfaces; their full
implementations remain outside this crate.

### 10. Cancellation and fetch lifecycle

**Evidence:** [`AbortSignal`](../src/api/cancellation.rs) is an atomic boolean plus notification, and transport/response streaming maps it to `RequestError::Aborted` in [`ReqwestTransport::send`](../src/transport/reqwest.rs) and [`Body::from_stream_with_signal`](../src/api/body/mod.rs).

Basic cancellation is implemented, including provider resolution and returned
provider streams: abort drops a pending provider future, terminates a provider
body with `RequestError::Aborted`, and body failure or drop releases its stream.
The blob, file, and page suites cover those lifecycle paths. Fetch controllers
still do not distinguish ongoing, terminated, and aborted states, preserve a
serialized abort reason, expose lifecycle callbacks, or provide
timing/reporting hooks. Cancellation during a redirect, cache revalidation,
cookie processing, or body capture is not modeled as a complete Fetch
algorithm state transition.

### 11. Response representation and Fetch API behavior

**Evidence:** [`Response`](../src/api/response/types.rs) exposes filtered status, URL, headers, response type, and body. The private internal representation retains full transport metadata. The crate still exports Rust structs and a stream, not JavaScript Fetch interfaces.

Remaining response work includes full redirect URL-list accuracy, response
constructors, a public response clone operation, Fetch-compatible stream
teeing, and complete network-error construction. `Body::array_buffer`,
`Body::form_data`, and `Body::json` exist for the supported Rust subset, but
their JavaScript-facing conversion belongs to `webapis`. Local scheme response
construction, provider metadata validation, and shared finalization are
implemented for the current provider-backed handlers. The supported Rust
subset and limitations are stated in
`fetch-engine-contract.md`.

### 12. Scheduling and transport coverage

**Evidence:** [`Request::transport_priority`](../src/api/request/definition.rs) maps Fetch low/auto/high priority to scheduler priority classes, but [`RequestScheduler::submit`](../src/scheduling/scheduler.rs) still receives `_priority` and never reorders admission. Reqwest is configured for Rustls, HTTP/2, compression, pooling, and a custom user agent in [`ReqwestTransport::new`](../src/transport/reqwest.rs).

The semaphore provides concurrency admission, and Fetch priority now reaches the scheduler boundary, but it still has no effect on ordering. There are no per-origin/host limits, network partition-aware connection pools, connection timing records, proxy policy, offline mode, or HTTP/3 transport. These are not all required for a minimal HTTP(S) Fetch subset, but they are gaps from the browser-wide behavior described by the standard and the crate's stated browser boundary.

### 13. Scheme-provider contract and local response lifecycle

**Evidence:** [`SchemeProviders`](../src/api/providers/mod.rs) holds optional,
thread-safe `BlobProvider`, `FileProvider`, and `PageProvider` capabilities;
[`RequestController::builder`](../src/engine/controller.rs) with its
`with_scheme_providers` setter installs them;
[`SchemeResource`](../src/api/providers/body.rs) carries the
provider-neutral URL, status, headers, origin, media type, and body; and
[`InternalResponse::from_scheme_resource`](../src/api/response/view.rs)
converts that data into the private response representation.

The boundary is present and the blob, file, page, and `about:blank` slices are implemented.
The provider API
defines stable categories for missing resources, revoked blobs, denied files,
provider I/O failure, malformed metadata, and invalid origins. Conversion
validates scheme-compatible URLs, provider metadata, and origin policy before
exposure. The handlers invoke only their matching installed providers, while
the engine-owned `about:blank` handler invokes none. Other `about:` URLs are
rejected. The browser installs its `byui:` provider, while concrete blob/file
providers and browser-wide `FetchServices` composition remain outside `net`.
Provider resolution and streams observe request cancellation, and focused
tests cover pending-resolution abort, stream abort/failure, and unread or
partially read body drop.

**Required work:** compose the remaining browser-owned providers and expand
the recording-provider conformance matrix across every provider-backed scheme.

## Recommended implementation order

1. Finish the browser-owned Feature 4.4 composition and provider conformance tests.
2. Expand remaining `data:`, blob, file, page, and `about:blank` lifecycle
   coverage.
3. Implement credentials/cookie policy and strict same-origin/CORS/no-CORS response filtering across HTTP and local schemes.
4. Complete per-hop redirect CORS/security and credential policy.
5. Replace the TTL map with a partitioned HTTP cache or integrate an existing standards-aware cache.
6. Complete shared Body cloning/teeing and JavaScript conversion semantics.
7. Add integrity, keepalive, service-worker, CSP/mixed-content, MIME/CORP, and timing hooks as the surrounding browser subsystems become available.

## Suggested regression-test matrix

At minimum, add tests for:

- forbidden methods/headers and no-CORS header filtering;
- same-origin, cross-origin simple CORS, failed CORS, credentialed CORS, and preflight caching;
- opaque and opaque-redirect responses;
- cookie `Set-Cookie` persistence, matching, expiry, SameSite, and credentials modes;
- redirect `follow`, `error`, and `manual`, including method/body rewriting and 20-hop limits;
- cache `default`, `no-store`, `reload`, `no-cache`, `force-cache`, and `only-if-cached`, including validators and `Vary`;
- `HEAD`, null-body statuses, body errors, cancellation reasons, SRI success/failure, and keepalive quotas;
- `data:`, `blob:`, `file:`, `byui:`, allowlisted `about:`, and unsupported-scheme behavior;
- provider absence, missing/revoked resources, denied access, malformed metadata, invalid origins, exact-once selection, and unrelated-provider/network exclusion;
- local-scheme cookie/cache exclusion, response filtering, empty/readable/null bodies, and cancellation during provider resolution or body streaming.
