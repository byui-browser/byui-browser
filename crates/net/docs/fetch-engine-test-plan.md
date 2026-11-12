# Net crate test plan

Status: the current request, header, response, cache, redirect, streaming,
scheduler, and scheme-handler regression suites pass. The remaining work is
conformance and integration coverage for behavior that is still partial or
owned by browser services. The 2026-10-21 `cargo llvm-cov -p net` baseline is
87.74% line coverage (3,164/3,552 executable lines); the gaps below are the
coverage-derived inventory that must remain represented by the work items in
this plan. Generated/debug-only formatting paths are included where they
exercise public behavior or stable error reporting.

## Coverage-derived gap inventory

The following concrete gaps are not optional cleanup; each must be covered by
the applicable section below or by a focused regression test when the behavior
is implemented:

- Body construction and lifecycle: `array_buffer`, URL-encoded construction,
  null/replayable/one-shot request-body conversion, `is_replayable`, known
  `length`, body equality/debug state, and the remaining multipart text/file
  serialization and malformed-part errors.
- Body and cancellation errors: cancellation defaults and state accessors,
  every stable `RequestError` display variant, transport/body conversion
  failures, and the distinction between body-already-used and
  body-already-consumed errors.
- Request/header API edges: request getters and setters, forbidden methods and
  headers, no-CORS method/content-type checks, body-not-allowed checks,
  keepalive size limits, redirect rewriting and sensitive-header stripping,
  all referrer-policy branches, header guard immutability/internal removal,
  default/invalid header parsing, and CORS-safelisted content types/ranges.
- Origin and response validation: unsupported/invalid origins, opaque origins,
  non-default ports, provider-origin mismatch/invalid-origin outcomes, and the
  response-type decisions for navigation and credentialed requests.
- HTTP execution: cache-mode branches, missing client-origin/cache
  revalidation/priority failures, URL joining and invalid joined URLs,
  pre-send and in-flight aborts, redirect limits and failures, non-replayable
  redirect bodies, method rewriting, and sensitive-header removal.
- Transport and scheduling: scheduler abort while waiting or immediately
  before transport, transport/client initialization failures, and transport
  body conversion failures.
- Data and response boundaries: malformed percent escapes, response policy
  failures, and controller/debug paths that must remain typed and side-effect
  free.

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
