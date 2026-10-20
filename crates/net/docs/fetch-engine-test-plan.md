# Net crate test plan

Status: the current request, header, response, cache, redirect, streaming,
scheduler, and scheme-handler regression suites pass. The remaining work is
conformance and integration coverage for behavior that is still partial or
owned by browser services.

## Remaining test work

### 1. Body state and response lifecycle

Add focused tests for:

- fallible response cloning and stream teeing once implemented;
- locked, disturbed, completed, failed, aborted, canceled, and dropped body
  transitions across direct readers and helper methods;
- known response length through transport, cache, and provider conversion;
- body behavior for network-error responses and `CONNECT`; and
- permit release and cache suppression after every body failure or abort.

### 2. Scheme-provider and browser composition

Extend the existing `tests/schemes/` coverage with:

- concrete browser composition through the builder's
  `with_fetch_services` and `with_scheme_providers` setters;
- exact-once matching-provider selection and request-context assertions;
- missing, revoked, denied, malformed, invalid-origin, and I/O outcomes;
- tuple and opaque provider origins;
- response metadata, filtering, media types, empty/readable/null bodies;
- cancellation during resolution and streaming, including provider cleanup;
- no HTTP transport, cookie/cache, or unrelated-provider side effects; and
- repeated `blob:`, `file:`, and `byui:` resolution without HTTP-cache reuse.

Add browser-owned tests for the embedded `byui:` provider covering route
aliases, unknown hosts/paths, query and fragment handling, `GET`/`HEAD`
semantics, repeated loads, body ownership, and origin policy.

### 3. CORS and response filtering

Complete tests for preflight requests, preflight caching, credentialed
preflight, redirect-time CORS checks, opaque and opaque-redirect behavior, and
network-error response construction. Retain explicit coverage proving that
forbidden response headers never reach public responses.

### 4. Redirects and request context

Add conformance cases for per-hop origin/referrer/credentials changes,
unsupported redirect targets, method/body rewriting, sensitive-header removal,
non-replayable bodies, URL-list accuracy, and the redirect limit.

### 5. Credentials, cookies, and cache

When browser providers exist, test partitioned cookie matching, persistence,
expiry, SameSite/secure behavior, credentials modes, `Set-Cookie` handling,
and CORS credential checks. Expand cache tests for `Vary`, freshness,
validators, 304 merging, unsafe-method invalidation, partitioning, and all
Fetch cache modes.

### 6. Policy, transport, and scheduling

Add tests for integrity, keepalive quotas, service-worker decisions,
CSP/mixed-content/CORP/MIME policy, offline behavior, timing hooks,
decompression limits and failures, range requests, explicit
`Accept-Encoding`, per-origin scheduling, and connection failures.

### 7. Unsupported-feature contract

Every intentionally unsupported feature must fail at its documented boundary,
before network/provider side effects, with a stable typed error. Update these
tests when a feature moves into the supported slice rather than deleting the
coverage.

## Verification

For each completed slice, run the focused net tests and relevant browser
tests. Before integration, run `cargo fmt --all`, workspace-relevant
`cargo test` commands, and `cargo clippy --all-targets -- -D warnings`.
