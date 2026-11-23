# Plan: make `net` the reusable Fetch engine

Status: the current engine boundary, headers, shared body consumption,
controller-owned HTTP redirects, local-scheme dispatch, provider contracts,
response filtering, cancellation, caching, and the builder-based construction
API are implemented for the supported Rust slice. The browser crate now
composes `BrowserFetchCapabilities`, the embedded `byui:` provider, and the
authorized `file:` provider. The file path is capability-based, bounded,
revocable, and cancellation-aware; it is not a general filesystem fallback.
The browser also exercises Fetch through page resource loading and JavaScript
integration. This document focuses on the remaining standards, browser-policy,
and lifecycle work rather than the provider plumbing that is now in place.

`net` owns reusable Fetch algorithms and transport-independent state.
`webapis` owns JavaScript bindings; browser, storage, and security crates own
durable state and browser policy. Those owners integrate through typed service
and provider interfaces rather than importing JavaScript types into `net`.

## Architecture and maintainability follow-up

Treat the following as internal refactors: preserve Fetch behavior and add
focused regression coverage before changing each area.

| Priority | Issue                                                                                                                                                                                                                                                                      | Potential solution                                                                                                                                                                                                              |
| -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Medium   | The process-local response cache covers bounded storage, structured identity, freshness, validators, revalidation, `304` merging, stale extensions, cache modes, and unsafe-method invalidation. Remaining gaps are complete HTTP/shared-cache policy and durable browser-owned storage. | Complete the remaining HTTP policy audit or delegate a standards-aware cache to the browser/storage owner. |
| Low      | Production cache and body-state locks still use `expect` on poisoning, so a poisoned lock becomes a panic rather than an explicit engine failure policy.                                                                                                                   | Map poisoned locks to typed errors where callers can recover, or document and test the deliberate process-failure policy for each remaining lock.                                                                               |
| Medium   | Lifecycle diagnostics stop at response headers. Body consumption, completion timing, cancellation timing, and transport/body failure details are not yet exposed through the browser-owned diagnostics boundary.                                                           | Extend the diagnostics contract with non-sensitive body and timing events, route transport failures through the redaction helpers, and add focused coverage for successful, failed, dropped, and cancelled streams.             |

Provider-backed `blob:`, `file:`, and `byui:` requests now use one declarative
dispatch path for method policy, provider selection, cancellation, and response
finalization. Do not reintroduce those steps in per-scheme handlers.

## Next implementation order

With the current browser/provider slice complete, the next work should be
sequenced as follows:

1. Wire browser identity, language, and Fetch Metadata policy into the
   completed per-hop generated-header and CORS-preflight boundary.
2. Replace the browser's conservative capability defaults with origin-aware
   cookies, service-worker decisions, security-policy checks, and durable
   cache partition state.
3. Decide whether `net` or the browser/storage owner should provide a
   standards-aware, durable HTTP cache, then complete the remaining shared-cache
   policy.
4. Add transport limits and complete diagnostics for body timing and
   cancellation, then expand conformance coverage.

The `byui:` and `file:` providers should receive maintenance fixes and
regression tests during these slices, but they are no longer the critical path.
Browser-owned `blob:` lifecycle work remains deferred.

## Remaining work

### 1. Complete remaining body integrations

Complete the JavaScript typed-array, `Request`, `Response`, promise, and stream
conversion surface in `webapis`. Add aggregate keepalive lifetime accounting
with the browser-lifecycle work in step 3.

### 2. Finish CORS, response, and request algorithms

- CORS preflight `OPTIONS` requests, credential validation, redirect-hop
  reevaluation, and a partitioned expiring permission cache are implemented.
  Redirect taint is derived from the complete URL list as same-origin,
  same-site, or cross-site; it controls origin serialization, credentials,
  preflight, and response filtering when a chain returns to its initial origin.
  Failed preflighted fetches clear only matching permission entries. The
  target-aware `Origin` algorithm applies redirect taint and referrer-policy
  reduction for CORS and non-CORS unsafe methods.
- Network-error response construction and response-derived helpers such as
  `ok` and explicit body-presence checks are implemented.
- Redirect rewrites remove the complete request-body header set, and referrer
  reduction is monotonic so a later policy cannot restore previously removed
  URL detail.
- Browser-generated request headers are assembled once per validated HTTP hop,
  cannot replace Fetch-owned origin, referrer, body, cookie, or preflight
  fields, and contribute CORS-unsafe names to the preflight request. This
  keeps redirect tainting and per-hop CORS approval aligned with the headers
  that reach transport.
- Destination-specific browser identity/language and Fetch Metadata headers,
  plus provider-owned cookie and authentication behavior, remain browser
  integrations in step 3 rather than CORS algorithm gaps in `net`.

### 3. Implement browser credentials and policy integrations

The current browser composition installs explicit no-op policy hooks: it sends
no cookies, stores no `Set-Cookie` values, always selects the network for
service workers, and uses a process-local cache partition. These defaults are
intentional for the current slice, but they are not browser-complete. Replace
them with browser-owned implementations for:

- partitioned cookie storage and `Set-Cookie` processing;
- TLS client certificates and HTTP authentication;
- CSP, mixed-content, Upgrade Insecure Requests, CORP, MIME/nosniff, and SRI;
- service-worker interception and bypass decisions;
- keepalive quotas and lifetime handling;
- offline/network state, complete timing coverage, and browser integrations for
  the existing diagnostics hooks; and
- browser client, destination, initiator, and navigation context.

Each hook must specify its fail-closed, unsupported, or default behavior when
no provider is installed.

### 4. Complete browser-owned cache integration

`net` now owns a process-local, transport-independent HTTP-cache subset and
response capture. Browser and storage layers continue to own durable
persistence and provide partition identity through replaceable capabilities.

The implemented cache:

- uses partition, method, normalized URL, credential mode and eligibility,
  request mode, redirect taint, and authorization presence in its structured
  identity;
- retains authorization presence without retaining authorization values and
  conservatively excludes authorized responses from storage;
- bounds primary entries and aggregate body bytes, evicts least-recently-used
  entries, and prunes stale entries;
- stores multiple `Vary` request-header variants, rejects `Vary: *`, and
  replaces variants when a response's `Vary` field set changes;
- parses `Date`, `Expires`, `Age`, `Last-Modified`, and supported freshness
  directives through a test-controlled clock;
- retains validators, sends conditional requests, merges successful `304`
  responses, and handles unsuccessful validation responses;
- supports stale-while-revalidate and stale-if-error in their documented
  windows;
- implements unsafe-method and same-origin location invalidation;
- implements `Default`, `NoStore`, `Reload`, `NoCache`, `ForceCache`, and
  `OnlyIfCached`;
- serves independent public body views from immutable buffered response bytes;
  and
- commits body capture only after clean, bounded end-of-stream completion.

Cache hits pass through the same response filtering and provider authorization
decisions as network responses.

Remaining cache work is to decide whether `net` or the browser/storage owner
will provide a durable standards-aware backend, then complete shared-cache
admission and freshness policy, complete HTTP admission and invalidation rules,
durable partition and persistence state, and browser cookie/authorization
integration.

### 5. Harden redirects and transport

Add explicit transport configuration and tests for decoded-body limits,
decompression ratios, malformed compressed bodies, range requests,
`Accept-Encoding`, connection failures, connection timing, per-origin limits,
offline mode, and future HTTP/3 support.

### 6. Scheme-provider/browser integration

#### Browser-owned composition

Extend the security-owned capability and browser adapter for authorized `file:`
access only when navigation/subresource policy or origin parsing is ready.
Preserve the rule that browser-owned providers fail closed rather than falling
back to HTTP or direct filesystem access when lookup fails.

#### Deferred `blob:` provider

Browser-owned blob URL storage, creator-origin association, revocation, and
lifetime management are deferred. The `net::BlobProvider` contract and generic
engine tests remain in place, but browser composition must not claim to provide
`blob:` URLs until a browser-owned registry and lifecycle tests exist.

#### `byui-provider` follow-up

Avoid a fresh `Vec` copy of embedded resource bytes for every request while
retaining the provider's current route, alias, origin, and `GET`/`HEAD`
semantics.

Add focused browser tests for method rejection, `GET`/`HEAD` bodies, query and
fragment handling, aliases, repeated loads, shared-body ownership, unknown
routes, and origin policy.

#### Provider conformance and isolation

Extend the existing browser-composition and `net` integration tests for newly
discovered regressions, including:

- builder-based service/provider composition and request-context propagation;
- exact-once selection of the matching provider;
- missing, revoked, denied, malformed, and I/O-failure outcomes;
- provider-assigned tuple and opaque origins;
- response headers, status, URL, media type, empty/readable/null bodies;
- cancellation while resolving and streaming, including resource cleanup;
- no HTTP transport, cookie, cache, or unrelated-provider calls; and
- repeated resolution of the same resource without accidental HTTP caching.

Add browser registry, creation, revocation, and lifetime tests when the
deferred `blob:` provider slice is resumed.

Any new local scheme must define its provider owner, origin policy, method
policy, body/null-body behavior, response filtering, and cache/cookie behavior
before implementation. Keep local schemes out of HTTP transport, cookies, and
cache unless a later contract explicitly changes that boundary.

#### Verification and documentation

Keep the Feature 4.4 contract, test plan, and standard-gap audit synchronized
with the implementation. Before considering browser-level scheme-provider
integration complete, run the focused `net` and browser tests, relevant
workspace tests, `cargo fmt --all`, and
`cargo clippy --all-targets -- -D warnings`.

### 7. Public API and downstream migration

Add request initialization APIs, a Rust-consumer guide, and documentation of
IPC-safe versus process-local types. Update the browser architecture
documentation and record the ownership decision in an ADR. Keep raw Reqwest
types private.

### 8. Conformance and verification

Select a realistic Web Platform Test subset after the engine APIs stabilize.
Track unsupported tests by owning subsystem and add regression tests for every
intentional limitation. Run `cargo fmt --all`, focused and workspace tests,
and `cargo clippy --all-targets -- -D warnings` for each integration slice.

### 9. Deferred WebSocket work

WebSocket support is not part of Fetch completion. Design it as a sibling
connection/session API with its own handshake policy, transport ownership,
backpressure, limits, cancellation, scheduler permits, and JavaScript
bindings after the HTTP Fetch work above is stable.

## Completion criteria

The engine is ready for browser-wide adoption when body ownership and cloning,
response filtering, redirects, credentials, cache behavior, policy hooks,
transport limits, provider composition, and documented unsupported boundaries
are covered by focused tests; browser consumers can integrate without
reimplementing networking or security semantics; and formatting, tests, and
clippy pass with warnings denied.
