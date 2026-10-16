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

Finish the Feature 4.4 work in
[`fetch-engine-feature-4-4-plan.md`](fetch-engine-feature-4-4-plan.md):
browser-owned `FetchServices`, blob/file ownership, `byui-provider` body-copy
and route hardening, origin policy, and browser-composition tests. Keep local
schemes out of HTTP transport, cookies, and cache unless a later contract
explicitly changes that boundary.

### 7. Public API and downstream migration

Add or finish documented Rust APIs for response cloning, body state where it
is intentionally public, response helpers, and request initialization. Add a
Rust-consumer guide and document IPC-safe versus process-local types. Update
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
