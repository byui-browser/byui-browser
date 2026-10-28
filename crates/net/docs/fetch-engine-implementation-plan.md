# Plan: make `net` the reusable Fetch engine

Status: the current engine boundary, headers, shared body consumption,
controller-owned HTTP redirects, local-scheme dispatch, provider contracts,
response filtering, and basic cancellation are implemented for the supported
Rust slice. This document now focuses on the work remaining to make that slice
more complete and browser-ready.

`net` owns reusable Fetch algorithms and transport-independent state.
`webapis` owns JavaScript bindings; browser, storage, and security crates own
durable state and browser policy. Those owners integrate through typed service
and provider interfaces rather than importing JavaScript types into `net`.

## Architecture and maintainability follow-up

Treat the following as internal refactors: preserve the documented Rust API and
Fetch behavior, and add focused regression coverage before changing each area.

| Priority | Issue                                                                                                                                                                                                  | Potential solution                                                                                                                                                                         |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| High     | `Body` combines source ownership, lifecycle state, cloning, polling, cancellation, tee buffering, and permit retention. `Clone` has different semantics for replayable and one-shot bodies.            | Separate ownership from lifecycle observation. Define one explicit cloning model, make source transfer fallible, and retain the bounded tee only if its backpressure contract is required. |
| High     | `Body::into_parts` can turn a previously transferred source into an empty stream, concealing repeated-consumption mistakes.                                                                            | Check lifecycle state and return a typed error for consumed or transferred sources, matching the transport handoff behavior.                                                               |
| High     | The HTTP fetch path combines cache lookup, transport exchange, redirects, CORS finalization, cookie handling, and streamed cache capture. Redirect handling also keeps the original scheme dispatch, processes cookies only for the final response, bypasses common response approval for manual redirects, and carries the original cacheability decision across method-rewriting redirects. | Split it into cache lookup, one exchange, redirect control, response finalization, and cache capture. Make each redirect hop revalidate its target and scheme, process intermediate response policy/cookies in order, finalize manual redirects through the common path, and recompute cache identity after method changes. Preserve ordering with integration tests. |
| High     | `FetchServices` is a broad capability interface combining request/response authorization, cookies, service workers, cache partitioning, and diagnostics. Most current browser implementations are no-op adapters, so ownership and failure behavior are difficult to see at call sites. | Split the interface into focused capability boundaries or an explicit services aggregate. Document which decisions are per request, per redirect hop, or per response, and define fail-closed behavior for every absent or unsupported capability. |
| Medium   | Provider-backed scheme handlers repeat the same method gate, provider lookup, cancellation wait, and response-finalization sequence across `blob:`, `file:`, and `byui:`. | Retain one common provider execution path and move scheme metadata, provider selection, and read-method policy into a declarative dispatch layer. Keep provider-specific ownership and error categories explicit. |
| Medium   | `Headers` mixes ordered storage, public mutations, guards, forbidden-header policy, CORS safelisting, exposure parsing, and transport conversion. Its public iteration repeatedly scans and allocates. | Split storage/API from guard and CORS policy; implement combined iteration in one pass and avoid temporary allocations in ordinary `get` calls.                                            |
| Medium   | `Request` and `InternalResponse` expose broadly mutable internal field bags. Request configuration, redirect state, cache partitioning, and transport state are intermixed.                            | Introduce caller-facing request initialization plus prepared/redirect execution state, and use narrow constructors and transition methods for internal responses.                          |
| Medium   | Request and response rules are distributed across setters, preparation, scheme handlers, the HTTP redirect loop, and response finalization. The same request service hook is invoked once during preparation and again for HTTP hops, while service-worker checks are only performed initially. | Introduce explicit request-preparation and per-hop validation states. Keep defense-in-depth checks, but centralize shared predicates and document which checks repeat after redirects so policy cannot drift between schemes. |
| Medium   | The process-local cache has an unbounded entry map, a narrow method/URL/partition key, and only partial `Cache-Control` support. It does not model `Vary`, authorization/credential state, validators, or revalidation. | Define the cache contract as an intentionally bounded adapter, add eviction and complete cache-key inputs, and either implement standards-aware freshness/revalidation or delegate those decisions to the browser cache owner. |
| Low      | `CachedResponse` mirrors `StoredResponse` and copies every field in both directions.                                                                                                                   | Store a `StoredResponse` with expiration metadata inside `CachedResponse`.                                                                                                                 |
| Low      | Crate-wide `allow(dead_code)`, production lock `expect` calls, and stale references to implementation-contract documentation hide unfinished work and can obscure the supported boundary. | Replace the allowance with narrow tracked exceptions; map poisoned locks to typed errors or explicitly document the intended failure policy. Keep the implementation plan, contract, and gap audit synchronized and verify all referenced documents exist. |

Do not reintroduce per-scheme response-finalization duplication that the shared
provider helper already removed.

## Remaining work

### 1. Complete body semantics

The Rust engine slice is implemented. `Body` has one authoritative lifecycle,
nullness is represented by the body itself, and helper reads, direct stream
reads, transport handoff, cache capture, failure, abort, cancellation, and drop
have focused transition tests. Declared lengths survive live HTTP, cache, and
provider paths. `Body::try_clone`, `Body::tee`, and `Response::try_clone`
provide fallible cloning with a bounded one-chunk tee, branch cancellation,
and shared scheduler-permit ownership. Cache capture has a configured size
bound and stores only cleanly completed bodies.

Multipart handling is binary-safe and supports RFC 5987 filenames. Rust
ArrayBuffer/typed-array constructors, half-duplex stream validation, UTF-8 JSON
decoding, response helpers, and a null-body network-error constructor are
implemented. `CONNECT` remains forbidden at request construction. The
JavaScript typed-array, `Request`, `Response`, promise, and stream conversion
surface remains in `webapis`; aggregate keepalive lifetime accounting remains
with the browser-lifecycle work in step 3.

### 2. Finish CORS, response, and request algorithms

- Implement preflight requests and the preflight cache.
- Complete `Access-Control-Allow-*` parsing, credentialed CORS, request
  tainting, and redirect-time policy checks.
- Finish network-error response construction and response-derived helpers such
  as `ok` and explicit body-presence checks.
- Complete user-agent-owned header generation and redirect-time header rules.
- Integrate structured origins, same-site checks, referrer changes, and
  credentials behavior across redirects and local schemes.

### 3. Implement browser credentials and policy integrations

Provide browser-owned implementations for the existing hooks:

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

Complete per-hop redirect policy for origin, referrer, credentials, CORS,
cache, and security decisions. Keep Reqwest redirects disabled and retain
controller ownership of URL-list state and method/body rewriting.

Add explicit transport configuration and tests for decoded-body limits,
decompression ratios, malformed compressed bodies, range requests,
`Accept-Encoding`, connection failures, connection timing, per-origin limits,
offline mode, and future HTTP/3 support.

### 6. Complete scheme-provider/browser integration

#### Browser-owned composition

Complete browser-side ownership and composition without moving browser state
into `net`:

- provide a concrete browser `FetchServices` implementation and compose it
  with `SchemeProviders` through the builder's `with_scheme_providers` setter
  when browser client context is available;
- keep `byui:` page registration, lookup, replacement, and origin policy in
  `crates/browser`;
- add browser/storage ownership for blob URL creation, creator origins,
  revocation, and lifetime; and
- add the designated filesystem/security owner for authorized `file:` access,
  sandboxing, path resolution, and origin policy.

Browser-owned providers must never fall back to HTTP or direct filesystem
access when lookup fails.

#### `byui-provider` hardening

Finish the provider-local implementation by avoiding a fresh `Vec` copy of
embedded resource bytes for every request. Retain the engine-enforced
`GET`/`HEAD` boundary and explicit `HEAD` body semantics. Keep route lookup
exact and declarative, including declared resource aliases and unknown-host or
path rejection, and document and enforce the chosen opaque or stable
browser-UI origin policy.

Add focused browser tests for method rejection, `GET`/`HEAD` bodies, query and
fragment handling, aliases, repeated loads, shared-body ownership, unknown
routes, and origin policy.

#### Provider conformance and isolation

Add browser-composition and `net` integration tests for:

- builder-based service/provider composition and request-context propagation;
- exact-once selection of the matching provider;
- missing, revoked, denied, malformed, and I/O-failure outcomes;
- provider-assigned tuple and opaque origins;
- response headers, status, URL, media type, empty/readable/null bodies;
- cancellation while resolving and streaming, including resource cleanup;
- no HTTP transport, cookie, cache, or unrelated-provider calls; and
- repeated resolution of the same resource without accidental HTTP caching.

Keep `data:` parsing engine-owned and retain the exact `about:blank`
allowlist. Unsupported schemes must fail before service, provider, cache, or
transport side effects. Any new local scheme must define its provider owner,
origin policy, method policy, body/null-body behavior, response filtering, and
cache/cookie behavior before implementation. Keep local schemes out of HTTP
transport, cookies, and cache unless a later contract explicitly changes that
boundary.

#### Verification and documentation

Keep the Feature 4.4 contract, test plan, and standard-gap audit synchronized
with the implementation. Before considering browser-level scheme-provider
integration complete, run the focused `net` and browser tests, relevant
workspace tests, `cargo fmt --all`, and
`cargo clippy --all-targets -- -D warnings`.

### 7. Public API and downstream migration

Make controller construction a single Fetch-oriented interface rather than a
four-constructor capability matrix. This is implemented by
`RequestController::builder(config)`, which provides optional `FetchServices`
and `SchemeProviders` setters and a fallible `build()` method. The former
specialized constructors have been removed, so callers use the builder and its
`with_fetch_services` and `with_scheme_providers` setters directly.

The builder must preserve the current capability rules: standalone requests
may omit services, local schemes require matching scheme providers, and
requests carrying browser client context require `FetchServices`. It should
also make the ownership distinction explicit without exposing controller
internals. Add constructor/builder tests for all capability combinations and
for failures before transport or provider side effects.

Then add or finish documented Rust APIs for response cloning, body state where
it is intentionally public, response helpers, and request initialization. Add
a Rust-consumer guide and document IPC-safe versus process-local types. Update
the browser architecture documentation and record the ownership decision in an
ADR. Keep raw Reqwest types private and migrate callers incrementally.

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
