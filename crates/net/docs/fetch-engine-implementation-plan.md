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

## Remaining work

### 1. Complete body semantics

- Replace compatibility body/null flags with one authoritative body model.
- Define and test every body state transition for helper reads, direct stream
  reads, transport handoff, cache capture, failure, abort, cancellation, and
  drop.
- Preserve response `Content-Length` metadata through live, cached, and
  provider-backed bodies.
- Implement fallible response/body cloning and a bounded stream tee with
  backpressure, branch cancellation, drop behavior, cache capture, and permit
  ownership rules.
- Finish binary-safe multipart parsing/serialization, RFC 5987 filenames,
  typed-array inputs, duplex semantics, JSON decoding edge cases, and the
  supported `CONNECT`/network-error body rules.

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
