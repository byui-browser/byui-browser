# Net crate test plan

Status as of 2026-11-10: the current body, request, header, response, cache, redirect,
streaming, scheduler, and scheme-handler regression suites pass. The first
body API/lifecycle test slice is now covered, including constructor metadata,
URL-encoded and ArrayBuffer consumption, equality/debug state, one-shot
transport consumption, failed/aborted streams, and malformed form data. The
body ownership slice now also covers explicit buffered and live forks,
bounded tee backpressure, response cloning for buffered, one-shot, null, and
live bodies, abort propagation to cloned branches, direct-reader/helper-future
drop cleanup, response/body length metadata, and cloned-stream cache and
scheduler-permit behavior. The
current redirect, scheduler, request-context, and unsupported-feature tests
also cover POST rewriting, sensitive-header stripping, non-replayable 307
requests, redirect limits, queued-request cancellation, credential/referrer
decisions, and service-side-effect boundaries. The
request-policy/transport hardening slice additionally covers forbidden
methods and headers, no-CORS body rules, keepalive size boundaries, origin
parsing and opaque-origin comparisons, relative URL resolution, connection
refusal, same-origin response rejection, and post-header abort cleanup. The
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

- Body construction and lifecycle: remaining direct-reader state transitions,
  known response length through any uncovered transport/cache paths, and the
  remaining multipart text/file serialization and malformed-part errors.
- Body and cancellation errors: cancellation defaults and accessors, every
  stable `RequestError` display variant, and the remaining transport/body
  conversion failures. One-shot transport consumption, body forking, stream
  failure/abort propagation, and the distinction between body-already-used
  and body-already-consumed are now covered.
- Request/header API edges: request getters and setters, forbidden methods and
  headers, remaining no-CORS method/content-type combinations, remaining
  request getter/setter coverage, header guard immutability/internal removal,
  default/invalid header parsing, and CORS-safelisted content types/ranges.
- Origin and response validation: provider-origin mismatch/invalid-origin
  outcomes, and the response-type decisions for navigation and credentialed
  requests.
- HTTP execution: cache-mode branches, missing client-origin/cache
  revalidation/priority failures, URL joining and invalid joined URLs,
  remaining pre-send/in-flight abort timing, remaining redirect failure paths,
  and redirect URL-list accuracy.
- Transport and scheduling: abort immediately before transport,
  client initialization failures, and transport body-conversion failures.
- Data and response boundaries: malformed percent escapes, response policy
  failures, and controller/debug paths that must remain typed and side-effect
  free.

## Remaining test work

### 1. Body state and response lifecycle

The initial body API/lifecycle slice is complete. `src/api/body/tests.rs` now
covers constructor state and length metadata, URL-encoded construction,
ArrayBuffer reads, equality and debug output, one-shot transport handoff,
failed and aborted streams, and malformed multipart inputs.

The body ownership slice covers explicit forks for buffered and live sources,
bounded tee backpressure, independent view status, checked internal transfer,
response cloning across buffered, one-shot, null, and live bodies, abort
propagation to cloned branches, direct-reader/helper-future source cleanup,
and suppression of cache insertion for disturbed or failed cloned sources.
Add further focused tests for:

- locked, disturbed, completed, canceled, and dropped body transitions across
  direct readers and helper methods;
- known response length through any uncovered transport and cache conversion;
- body behavior for network-error responses; and
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
ownership and length metadata, revocation, limits, and origin policy as those
integrations evolve. The net scheme suites now also verify provider response
lengths, stream/null-body length semantics, shared response finalization, and
provider cleanup after unread or partially read bodies.

Browser blob registry, creation, revocation, and lifetime tests are deferred.
Keep `net/tests/schemes/blob.rs` as generic provider-contract regression
coverage, without adding browser composition requirements to this slice.

### 3. CORS and response filtering

Preflight request construction, permission denial, cache reuse and clearing,
credentialed preflight, opaque-origin serialization, wildcard restrictions,
cookie suppression, and multi-hop redirect preflight are covered. This includes
the non-credentialed wildcard success case, the `Authorization` wildcard
exception, and the rule that a credentialed preflight never carries cookies
while its eventual request may. Opaque local `data:` origins are rejected for
tuple-origin CORS clients. Header unit tests also cover CORS-safelisted request
headers, unsafe-name sorting/deduplication and the aggregate limit, response
header exposure, credential-sensitive exposure wildcards, malformed exposure
values, and forbidden response-header filtering. Redirect-time simple CORS,
redirect-tainted origin serialization, and opaque-redirect approval are
covered. URL-list-derived same-origin, same-site, and cross-site taint have unit
coverage. A redirect chain that leaves and returns to its request origin proves
that `null` origin serialization, credential suppression, CORS filtering, and
preflight remain active at the returning hop.

Preflight failure coverage now includes non-success and redirect statuses,
duplicated allow-origin fields, case-sensitive method tokens, and missing
credential approval. An approved preflight followed by a denied actual
response proves that no response is exposed and that only its matching cache
identity is evicted. Max-age parsing/default/capping and zero-lifetime expiry
have unit coverage, while a configured zero cap is also exercised end to end.
A 302 POST-to-GET rewrite proves that removed body metadata does not cause an
unnecessary target preflight.

The CORS suite also verifies that an opaque local `data:` origin is rejected
before response policy or exposure, a CORS-approved response keeps a later
body-stream failure typed, and a later redirect target that denies preflight
never receives its actual request.

Generated request-header coverage now includes the service-owned per-hop hook
and verifies that a forbidden Fetch Metadata field is named in
`Access-Control-Request-Headers` before it reaches a cross-origin target.
Remaining browser tests should verify destination, language, identity, and
Fetch Metadata values plus their redirect-time regeneration or removal.
`fetch` retains typed Rust errors for diagnostics, while `fetch_response`
provides the response-shaped network-error boundary.

Remaining CORS tests are defense-in-depth rather than missing algorithms:

- End-to-end permission-key isolation across origins, URL paths, network
  partitions, and credential modes (the cache has direct unit coverage).
- Response-shaped network errors for transport, CORS, redirect, provider, and
  cache-only failures, with abort remaining distinguishable and body-stream
  failures remaining errors on the delivered body.

### 4. Redirects and request context

The redirect and request-context slice covers the redirect status/method matrix,
content-header removal, cross-origin sensitive-header stripping, replayable and
non-replayable body behavior, the redirect limit, HTTP(S)-only and
credential-free targets, per-hop browser policy, service-worker bypass, intermediate
cookie processing, simple CORS checks, redirect-taint serialization, referrer-policy
updates, redirect-aware cache lookup, and representative credential decisions.

Add conformance cases alongside future same-site cookie, full-cache,
mixed-content, and timing integrations. Public final-URL and redirect-count
behavior is covered; expand internal URL-list assertions if that state becomes
observable through diagnostics.

### 5. Credentials, cookies, and cache

The cache unit suite now covers `Vary` header normalization, matching request
header values, multiple variants, `Vary: *` rejection, and replacement when
the response's `Vary` field set changes. When browser providers exist, test
partitioned cookie matching, persistence,
expiry, SameSite/secure behavior, credentials modes, `Set-Cookie` handling,
and CORS credential checks. Expand cache tests for freshness, validators, 304
merging, unsafe-method invalidation, partitioning, and all Fetch cache modes.

### 6. Policy, transport, and scheduling

The current tests prove integrity, cache-revalidation, priority, keepalive,
navigation requests fail before browser-service or network
side effects. They also prove service-worker interception fails before cache,
cookie, response, or transport work, and that canceling a scheduler-queued
request releases admission for the next request.

The hardening slice also covers forbidden request methods/headers, no-CORS
content-type and range safelisting, known keepalive body-size boundaries,
origin parsing, relative URL resolution, connection refusal, same-origin
response rejection, and aborting a delivered HTTP response before its body is
read. Failed, dropped, and aborted HTTP bodies now each have a regression
proving that their scheduler permit is released and their response is not
cached; cloned-response failure and abort paths have the same coverage.

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
