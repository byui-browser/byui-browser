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

## Next implementation order

With the current browser/provider slice complete, the next work should be
sequenced as follows:

1. Finish body ownership and cloning semantics, including tee/backpressure and
   cache capture, so every consumer can safely share or discard a response.
2. Implement the missing CORS preflight/cache and complete per-hop redirect
   policy before expanding browser credentials.
3. Replace the browser's no-op `FetchServices` defaults with origin-aware
   cookies, service-worker decisions, and security-policy checks.
4. Expand the process-local cache into a standards-aware, partitioned cache
   with freshness, validators, revalidation, and 304 handling.
5. Add transport limits and diagnostics, then broaden conformance coverage.

The `byui:` and `file:` providers should receive maintenance fixes and
regression tests during these slices, but they are no longer the critical path.
Browser-owned `blob:` lifecycle work remains deferred.

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

Complete per-hop redirect policy for origin, referrer, credentials, CORS,
cache, and security decisions. Keep Reqwest redirects disabled and retain
controller ownership of URL-list state and method/body rewriting.

Add explicit transport configuration and tests for decoded-body limits,
decompression ratios, malformed compressed bodies, range requests,
`Accept-Encoding`, connection failures, connection timing, per-origin limits,
offline mode, and future HTTP/3 support.

### 6. Scheme-provider/browser integration

#### Completed in the current slice

The engine-owned `data:` and exact `about:blank` paths, plus the browser-owned
`byui:` and authorized `file:` providers, now have end-to-end composition and
focused tests. The current coverage includes provider selection, method
boundaries, response metadata validation, readable/null bodies, streaming,
abort and drop cleanup, missing/revoked capabilities, file size and session
budgets, media-type policy, and fail-closed behavior. Browser page loading now
resolves its HTML, CSS, and JavaScript resources through the `byui:` provider.

Do not add browser blob URL creation, registry, revocation, or lifetime
behavior until the browser-owned blob lifecycle is designed.

#### Browser-owned composition and follow-up

Keep browser-side ownership and composition out of `net`:

- keep `byui:` page registration, lookup, replacement, and origin policy in
  `crates/browser`;
- retain the security-owned capability and the browser adapter for authorized
  `file:` access, sandboxing, path resolution, and origin policy; extend them
  only when navigation/subresource policy or origin parsing is ready.

Browser-owned providers must never fall back to HTTP or direct filesystem
access when lookup fails.

#### Deferred `blob:` provider

Browser-owned blob URL storage, creator-origin association, revocation, and
lifetime management are deferred. The `net::BlobProvider` contract and generic
engine tests remain in place, but browser composition must not claim to provide
`blob:` URLs until a browser-owned registry and lifecycle tests exist.

#### `byui-provider` follow-up

The current provider has exact declarative route lookup, compile-time embedded
assets, aliases, unknown-route rejection, opaque origins, and the engine's
`GET`/`HEAD` boundary. The remaining optimization is to avoid a fresh `Vec`
copy of embedded resource bytes for every request while retaining those
semantics.

Add focused browser tests for method rejection, `GET`/`HEAD` bodies, query and
fragment handling, aliases, repeated loads, shared-body ownership, unknown
routes, and origin policy.

#### Provider conformance and isolation

Keep the existing browser-composition and `net` integration tests for the
implemented `data:`, `byui:`, `file:`, and `about:blank` paths, and extend them
only for newly discovered regressions:

- builder-based service/provider composition and request-context propagation;
- exact-once selection of the matching provider;
- missing, revoked, denied, malformed, and I/O-failure outcomes;
- provider-assigned tuple and opaque origins;
- response headers, status, URL, media type, empty/readable/null bodies;
- cancellation while resolving and streaming, including resource cleanup;
- no HTTP transport, cookie, cache, or unrelated-provider calls; and
- repeated resolution of the same resource without accidental HTTP caching.

Keep the existing generic `blob:` provider contract tests as regression
coverage, but defer browser registry, creation, revocation, and lifetime tests
until the `blob:` provider slice is resumed.

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

Controller construction is now a single Fetch-oriented interface rather than
a four-constructor capability matrix. It is implemented by
`RequestController::builder(config)`, which provides optional `FetchServices`
and `SchemeProviders` setters and a fallible `build()` method. The former
specialized constructors have been removed, so callers use the builder and its
`with_fetch_services` and `with_scheme_providers` setters directly.

The current builder preserves these capability rules: standalone requests
may omit services, local schemes require matching scheme providers, and
requests carrying browser client context require `FetchServices`. It should
also make the ownership distinction explicit without exposing controller
internals. The existing constructor/builder tests cover all capability
combinations and verify failures before transport or provider side effects.

The builder migration and capability-combination tests are complete. Remaining
work is to add or finish documented Rust APIs for response cloning,
body state where it is intentionally public, response helpers, and request
initialization. Add a Rust-consumer guide and document IPC-safe versus
process-local types. Update the browser architecture documentation and record
the ownership decision in an ADR. Keep raw Reqwest types private.

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
