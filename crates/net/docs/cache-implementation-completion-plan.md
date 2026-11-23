# Net Cache Implementation Completion Plan

## Current state

The `net` crate has a working process-local cache foundation, but it is still
a partial Fetch HTTP-cache implementation. It currently provides:

- structured cache identity for partition, method, normalized URL, credential
  eligibility, request mode, redirect taint, and authorization presence;
- fail-closed effective-header authorization admission and browser partition
  selection derived from the request's network partition key;
- bounded storage with primary-entry and aggregate-body-byte limits;
- least-recently-used eviction and stale-entry pruning;
- multiple `Vary` request-header variants, including `Vary: *` rejection and
  variant replacement when the `Vary` field set changes;
- `Vary` matching against the complete outgoing header set, including
  Fetch-generated, policy-generated, body-metadata, and cookie fields;
- explicit freshness based on `Date`, `Expires`, `Age`, and supported
  `Cache-Control` directives, including stale extensions, plus conservative
  `Last-Modified` heuristic freshness when explicit metadata is absent;
- regenerated `Age` metadata whenever a stored response is reused;
- `stale-while-revalidate` with one detached refresh per matching variant and
  `stale-if-error` fallback for transport and 5xx failures;
- validator retention, conditional requests, `304 Not Modified` freshening and
  validation-response side effects, and unsafe-request invalidation;
- independent public body readers sharing immutable cached body bytes; and
- response filtering and provider authorization on cache hits.

This is intentionally a documented subset, not yet a complete Fetch HTTP
cache. See the [Fetch standard gap audit](net-fetch-standard-gaps.md#7-http-cache-and-cache-modes)
and [implementation plan](fetch-engine-implementation-plan.md#4-replace-the-process-local-cache-subset)
for the source requirements.

## Remaining implementation work

### 1. Establish the cache ownership boundary

Decide whether `net` will expand its adapter into the standards-aware HTTP
cache or whether the browser/storage layer will provide that implementation.
Regardless of the choice, the boundary must preserve `net`'s transport-
independent cache matching and response-capture behavior while allowing the
browser to supply partition identity and durable persistence.

### 2. Finish cache identity and matching

The current identity is conservative rather than Fetch-complete. It includes
the browser-provided partition, method, normalized URL, credential eligibility,
request mode, redirect taint, authorization presence, and `Vary` variants.
Authorization values are never retained and authorized requests are excluded
from storage.

Remaining work:

- Revisit the extra cache-key dimensions. They conservatively reduce reuse but
  do not model HTTP cache matching directly.
- Extend the current status-admission subset and cached-redirect behavior to
  the complete HTTP caching rules. The cache now admits supported
  default-cacheable response statuses plus other responses with explicit
  cache metadata, rejects `206 Partial Content` until range-aware completeness
  tracking exists, and follows stored redirects for cache hits (including
  `OnlyIfCached`), but the status policy remains intentionally incomplete.

Freshness uses explicit `Cache-Control`, `Date`, `Expires`, and `Age` metadata
where available. If no explicit lifetime is present, a valid `Last-Modified`
date supplies a conservative one-tenth apparent-age heuristic.

### 3. Finish conditional revalidation

Validator-aware revalidation retains and sends both `ETag` and `Last-Modified`
when available. A `304` replaces every stored instance of its supplied
representation fields, removes stale `1xx` warning fields, preserves retained
body metadata, refreshes freshness and validator state, and updates the stored
`Vary` request snapshot. Changed `Vary` field sets remove obsolete variants;
`Vary: *` and `no-store` serve the validated representation to the current
caller but remove it from reusable storage. Validation-response `Set-Cookie`
fields reach browser storage without being retained for later cache hits, and
approved validation headers reach the browser response-policy hook for other
browser-owned side effects. A validation response without a stored validator
replaces the representation normally, and an unsuccessful validation response
leaves the existing representation intact. A late `304` cannot freshen a
representation that a concurrent revalidation has already replaced; that
caller uses the newer cached representation.

`CacheMode::NoCache` sends `Cache-Control: max-age=0` when the caller did not
already provide `Cache-Control`, matching Fetch's request-header preparation
rule.

### 4. Finish cache invalidation

Unsafe methods invalidate every cached representation for the request URL in
the active cache partition, including method and `Vary` variants. A successful
2xx or 3xx response also invalidates a same-origin `Location` target. URL
fragments are ignored, while unrelated URLs, origins, and partitions remain
untouched.

Focused integration coverage verifies that successful 2xx and 3xx unsafe
responses invalidate the request target and only same-origin `Location`
targets, while unsuccessful responses preserve both targets. Unit coverage
preserves partition and `Vary`-variant isolation. Remaining work is an audit
against the complete HTTP invalidation rules before introducing a durable
backend.

### 5. Finish all Fetch cache modes

Complete the behavior selected by each cache mode:

- `Default`: use the normal freshness, matching, and revalidation algorithm;
- `NoStore`: bypass lookup and do not store the response;
- `Reload`: bypass an existing entry, send the required reload directives,
  and allow the refreshed response to replace the entry;
- `NoCache`: revalidate matching entries before use;
- `ForceCache`: use a matching stored response regardless of freshness;
- `OnlyIfCached`: enforce the same-origin restriction and return a Fetch
  network error without network access on a miss.

The current implementation has all six modes represented. `Reload` and
`NoStore` add `Pragma: no-cache` and `Cache-Control: no-cache` only when the
caller did not already provide each respective field; `NoCache` likewise
preserves a caller-provided `Cache-Control` field. Default requests with
caller-supplied validators use `no-store` behavior. Caller-supplied request
`no-cache` triggers validation in default mode, and request `no-store` bypasses
lookup and storage. These are Fetch and HTTP cache-control rules rather than
known deviations.

Cached redirects are stored and followed for `OnlyIfCached` when redirect mode
is `Follow`. `OnlyIfCached` outside same-origin mode currently returns the
Rust `NetworkError` before network I/O. The JavaScript `fetch` binding rejects
that invalid option combination with the `TypeError` required during request
construction.

### 6. Revisit body-capture semantics

The cache follows bounded streaming-capture semantics. A cacheable response
body is teed: the caller receives one branch and a background capture branch
collects the representation with one queued chunk per branch. Disturbed
bodies cannot be captured; failed or aborted streams, and bodies exceeding the
configured per-entry limit, are discarded. Only clean, bounded end-of-stream
capture inserts a response.

The dropped-caller-body regression now passes. Remaining work is to decide
whether delaying insertion until end-of-stream is acceptable: the Fetch/HTTP
cache algorithm can make a response available to later requests while the
original body is still being delivered, whereas this implementation does not.

### 7. Complete HTTP freshness policy

The cache now supports `stale-while-revalidate` and `stale-if-error`. Default
requests serve a stale matching response during its stale-while-revalidate
window and share one detached `NoCache` refresh for that variant. A stale
response in its stale-if-error window can mask transport and 5xx failures.
`must-revalidate` and `no-cache` disable both extensions. `ForceCache` and
`OnlyIfCached` continue to use any matching stored representation regardless
of freshness.

This process-local browser cache is private to its selected partition:
`public`, `private`, and `immutable` are accepted without changing normal
fresh-cache behavior, while `s-maxage` is parsed but does not replace
`max-age`. A shared durable backend will need its own admission and s-maxage
policy. Remaining work includes more complete heuristic freshness rules and
documenting the durable-backend policy.

## Browser and storage integrations

Completing the `net` cache algorithm does not by itself provide a browser-
complete cache. The browser/storage layer still needs to supply, as applicable:

- durable cache persistence and network partition state;
- browser-owned partition keys;
- cookie and `Set-Cookie` processing coordinated with credentials;
- authorization and authentication policy; and
- security-policy decisions that affect cache use or response exposure.

These integrations must preserve the existing fail-closed behavior when a
required provider is absent.

## Verification requirements

Add focused unit and integration coverage for:

- explicit and heuristic freshness, including `Date`, `Expires`, `Age`, and
  `Cache-Control` combinations;
- validator selection and conditional requests;
- `304` merging and refreshed cache metadata;
- unsafe-method invalidation and partition isolation;
- credentials, authorization, response taint, and `Vary` matching;
- every Fetch cache mode, including same-origin `OnlyIfCached`; and
- body capture after clean completion, failure, cancellation, dropping, and
  size-limit rejection.

Add focused coverage for:

- validation failure, missing validators, changed validators, revalidation,
  concurrent replacement, and `Set-Cookie` during `304`;
- generated-header-aware `Vary` matching;
- cacheable non-2xx responses and cached redirects; and
- missing browser cache partitions and the resulting `no-store` behavior;
- effective generated authorization, request cache directives, generated
  `Age`, and partial-response rejection.

Run the focused `net` tests together with formatting and clippy before the
cache work is considered complete.
