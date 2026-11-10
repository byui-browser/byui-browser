# Plan: make `net` the reusable Fetch engine

Status: the current engine boundary, headers, shared body consumption,
controller-owned HTTP redirects, local-scheme dispatch, provider contracts,
response filtering, cancellation, caching, and the builder-based construction
API are implemented for the supported Rust slice. The browser crate now
composes `BrowserFetchServices`, the embedded `byui:` provider, and the
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
focused regression coverage before changing each area. The body ownership
slice explicitly replaces implicit `Body` and `Request` cloning with fallible
operations, so Rust callers must migrate those calls.

| Priority | Issue                                                                                                                                                                                                                                                                                           | Potential solution                                                                                                                                                                                                                                         |
| -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| High     | `FetchServices` is a broad capability interface combining request/response authorization, cookies, service workers, cache partitioning, and diagnostics. Most current browser implementations are no-op adapters, so ownership and failure behavior are difficult to see at call sites.         | Split the interface into focused capability boundaries or an explicit services aggregate. Document which decisions are per request, per redirect hop, or per response, and define fail-closed behavior for every absent or unsupported capability.         |
| Medium   | Provider-backed scheme handlers repeat the same method gate, provider lookup, cancellation wait, and response-finalization sequence across `blob:`, `file:`, and `byui:`.                                                                                                                       | Retain one common provider execution path and move scheme metadata, provider selection, and read-method policy into a declarative dispatch layer. Keep provider-specific ownership and error categories explicit.                                          |
| Medium   | `Headers` mixes ordered storage, public mutations, guards, forbidden-header policy, CORS safelisting, exposure parsing, and transport conversion. Its public iteration and lookup repeatedly scan and allocate, including an effectively O(n²) combination path.                                | Split storage/API from guard and CORS policy; implement combined iteration in one pass and avoid temporary allocations in ordinary `get` calls.                                                                                                            |
| Medium   | `Request` and `InternalResponse` expose broadly mutable internal field bags. Request configuration, redirect state, cache partitioning, and transport state are intermixed.                                                                                                                     | Introduce caller-facing request initialization plus prepared/redirect execution state, and use narrow constructors and transition methods for internal responses.                                                                                          |
| Medium   | Request and response rules are distributed across setters, preparation, scheme handlers, the HTTP redirect loop, and response finalization. The same request service hook is invoked once during preparation and again for HTTP hops, while service-worker checks are only performed initially. | Introduce explicit request-preparation and per-hop validation states. Keep defense-in-depth checks, but centralize shared predicates and document which checks repeat after redirects so policy cannot drift between schemes.                              |
| Medium   | The process-local cache has an unbounded entry map, a narrow method/URL/partition key, and only partial `Cache-Control` support. It does not model `Vary`, authorization/credential state, validators, or revalidation.                                                                         | Define the cache contract as an intentionally bounded adapter, add eviction and complete cache-key inputs, and either implement standards-aware freshness/revalidation or delegate those decisions to the browser cache owner.                             |
| Low      | `CachedResponse` mirrors `StoredResponse` and copies every field in both directions. Cache insertion and cache hits also clone complete response bodies, which can create avoidable memory bandwidth and peak allocations for large entries.                                                    | Store a `StoredResponse` with expiration metadata inside `CachedResponse`, and use shared or move-based body ownership where the cache contract permits without exposing mutable state.                                                                    |
| Low      | Production lock `expect` calls and stale references to implementation-contract documentation hide unfinished work and can obscure the supported boundary.                                                                                                                                       | Replace the allowance with narrow tracked exceptions; map poisoned locks to typed errors or explicitly document the intended failure policy. Keep the implementation plan, contract, and gap audit synchronized and verify all referenced documents exist. |
| Medium   | Trace/debug paths can log raw response URLs and transport error strings. Transport diagnostics may contain full URLs, credentials, or other sensitive URL components, bypassing the crate's redaction helpers.                                                                                  | Route every URL and transport error through the existing redaction helpers, add regression coverage for logging-safe formatting, and keep public diagnostic strings explicitly non-sensitive where possible.                                               |

Do not reintroduce per-scheme response-finalization duplication that the shared
provider helper already removed.

## Next implementation order

With the current browser/provider slice complete, the next work should be
sequenced as follows:

1. Complete the remaining user-agent header algorithms around the implemented
   request-tainting and per-hop CORS preflight boundaries.
2. Replace the browser's no-op `FetchServices` defaults with origin-aware
   cookies, service-worker decisions, and security-policy checks.
3. Expand the process-local cache into a standards-aware, partitioned cache
   with freshness, validators, revalidation, and 304 handling.
4. Add transport limits and diagnostics, then broaden conformance coverage.

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
  Complete broader user-agent-owned header generation. Simple CORS and origin
  transitions run at each redirect hop.
- Finish network-error response construction and response-derived helpers such
  as `ok` and explicit body-presence checks.
- Complete user-agent-owned header generation and redirect-time header rules.
- Complete remaining referrer and credentials behavior across redirects and
  local schemes using the implemented structured-origin and schemeful-site
  checks.

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
- offline/network state, timing, and diagnostics hooks; and
- browser client, destination, initiator, and navigation context.

Each hook must specify its fail-closed, unsupported, or default behavior when
no provider is installed.

### 4. Replace the process-local cache subset

Define whether the browser supplies a standards-aware cache or `net` expands
its adapter. The resulting implementation must cover:

- `Vary`, credentials, authorization, response tainting, and partition keys;
- `Date`, `Expires`, `Age`, `Cache-Control`, freshness, and revalidation;
- validators, conditional requests, 304 merging, and unsafe-method
  invalidation; and
- all Fetch cache modes, including same-origin `OnlyIfCached` behavior.

Cache hits must create an independent public body view and must pass through
the same response filtering and provider authorization decisions as network
responses.

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
