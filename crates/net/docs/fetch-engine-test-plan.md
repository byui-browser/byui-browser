# Net crate test plan

Status: the current body, request, header, response, cache, redirect,
streaming, scheduler, and scheme-handler regression suites pass. The first
body API/lifecycle test slice is now covered, including constructor metadata,
URL-encoded and ArrayBuffer consumption, equality/debug state, one-shot
transport consumption, failed/aborted streams, and malformed form data. The
current redirect, scheduler, request-context, and unsupported-feature tests
also cover POST rewriting, sensitive-header stripping, non-replayable 307
requests, redirect limits, queued-request cancellation, credential/referrer
decisions, and service-side-effect boundaries. The
remaining work is conformance and integration coverage for behavior that is
still partial or owned by browser services. The last recorded `cargo llvm-cov -p net` baseline
is from 2026-10-21 (87.74% line coverage, 3,164/3,552 executable lines); it
must be refreshed when coverage is used to reprioritize this inventory.
Generated/debug-only formatting paths are included where they exercise public
behavior or stable error reporting.

## Coverage-derived gap inventory

The following concrete gaps are not optional cleanup; each must be covered by
the applicable section below or by a focused regression test when the behavior
is implemented:

- Body construction and lifecycle: remaining null/replayable/stream request
  transport-conversion cases, direct-reader state transitions, known response
  length through transport/cache/provider conversion, and the remaining
  multipart text/file serialization and malformed-part errors.
- Body and cancellation errors: cancellation defaults and accessors, every
  stable `RequestError` display variant, and the remaining transport/body
  conversion failures. One-shot transport consumption and the distinction
  between body-already-used and body-already-consumed are now covered.
- Request/header API edges: request getters and setters, forbidden methods and
  headers, no-CORS method/content-type checks, body-not-allowed checks,
  keepalive size limits, remaining redirect rewriting cases, remaining
  referrer-policy branches, header guard immutability/internal removal,
  default/invalid header parsing, and CORS-safelisted content types/ranges.
- Origin and response validation: unsupported/invalid origins, opaque origins,
  non-default ports, provider-origin mismatch/invalid-origin outcomes, and the
  response-type decisions for navigation and credentialed requests.
- HTTP execution: cache-mode branches, missing client-origin/cache
  revalidation/priority failures, URL joining and invalid joined URLs,
  pre-send and in-flight aborts, remaining redirect failure paths, and
  redirect URL-list accuracy.
- Transport and scheduling: abort immediately before transport,
  transport/client initialization failures, and transport body-conversion
  failures.
- Data and response boundaries: malformed percent escapes, response policy
  failures, and controller/debug paths that must remain typed and side-effect
  free.

## Remaining test work

### 1. Body state and response lifecycle

The initial body API/lifecycle slice is complete. `src/api/body/tests.rs` now
covers constructor state and length metadata, URL-encoded construction,
ArrayBuffer reads, equality and debug output, one-shot transport handoff,
failed and aborted streams, and malformed multipart inputs.

Add focused tests for:

- fallible response cloning and stream teeing once implemented;
- locked, disturbed, completed, canceled, and dropped body transitions across
  direct readers and helper methods;
- known response length through transport, cache, and provider conversion;
- body behavior for network-error responses and `CONNECT`; and
- permit release and cache suppression after every body failure or abort.

### 2. Scheme-provider and browser composition

The net crate already covers the generic provider contract and lifecycle paths
in `tests/schemes/`. Extend that coverage for the currently composed providers,
and add browser-owned tests, with:

- browser composition through `browser::request_controller`, including the
  current `byui:` and authorized `file:` providers;
- exact-once matching-provider selection and request-context assertions;
- missing, revoked, denied, malformed, invalid-origin, and I/O outcomes;
- tuple and opaque provider origins;
- response metadata, filtering, media types, empty/readable/null bodies;
- cancellation during resolution and streaming, including provider cleanup;
- no HTTP transport, cookie/cache, or unrelated-provider side effects; and
- repeated `file:` and `byui:` resolution without HTTP-cache reuse;
- `data:` decoding, `about:blank`, and unsupported-scheme side-effect checks.

The browser-owned provider tests cover the embedded `byui:` routes and the
authorized file provider; keep expanding route aliases, unknown hosts/paths,
query and fragment handling, `GET`/`HEAD` semantics, repeated loads, body
ownership, revocation, limits, and origin policy as those integrations evolve.

Browser blob registry, creation, revocation, and lifetime tests are deferred.
Keep `net/tests/schemes/blob.rs` as generic provider-contract regression
coverage, without adding browser composition requirements to this slice.

### 3. CORS and response filtering

Complete tests for preflight requests, preflight caching, credentialed
preflight, redirect-time CORS checks, opaque and opaque-redirect behavior, and
network-error response construction. Retain explicit coverage proving that
forbidden response headers never reach public responses.

### 4. Redirects and request context

The initial redirect and request-context slice now covers POST-to-GET
rewriting, content-header removal, cross-origin sensitive-header stripping,
non-replayable 307 rejection, the redirect limit, tuple/opaque credential
decisions, and representative same-origin, cross-origin, and downgrade
referrer behavior.

Add conformance cases for per-hop origin/referrer/credentials changes,
unsupported redirect targets, remaining redirect status/method combinations,
and URL-list accuracy.

### 5. Credentials, cookies, and cache

When browser providers exist, test partitioned cookie matching, persistence,
expiry, SameSite/secure behavior, credentials modes, `Set-Cookie` handling,
and CORS credential checks. Expand cache tests for `Vary`, freshness,
validators, 304 merging, unsafe-method invalidation, partitioning, and all
Fetch cache modes.

### 6. Policy, transport, and scheduling

The current tests prove integrity, cache-revalidation, priority, keepalive,
navigation, and preflight requests fail before browser-service or network
side effects. They also prove service-worker interception fails before cache,
cookie, response, or transport work, and that canceling a scheduler-queued
request releases admission for the next request.

Add tests for keepalive quotas, service-worker decisions beyond interception,
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
