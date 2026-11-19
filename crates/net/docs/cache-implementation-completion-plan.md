# Net Cache Implementation Completion Plan

## Current state

The `net` crate has a working process-local cache foundation, but it is still
a partial Fetch HTTP-cache implementation. It currently provides:

- structured cache identity for partition, method, normalized URL, credential
  eligibility, request mode, redirect taint, and authorization presence;
- bounded storage with primary-entry and aggregate-body-byte limits;
- least-recently-used eviction and stale-entry pruning;
- multiple `Vary` request-header variants, including `Vary: *` rejection and
  variant replacement when the `Vary` field set changes;
- explicit freshness based on `Date`, `Expires`, `Age`, and supported
  `Cache-Control` directives, plus conservative `Last-Modified` heuristic
  freshness when explicit metadata is absent;
- validator retention, conditional requests, basic `304 Not Modified`
  merging, and unsafe-request invalidation;
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

- Make the cache partition mandatory for browser-context requests. The current
  standalone fallback can create an unpartitioned cache rather than selecting
  `no-store` when no browser HTTP cache is available.
- Match `Vary` against the complete outgoing request, including generated
  headers such as `Origin`, `Referer`, user-agent/Fetch Metadata fields, body
  metadata, and cookies, not only caller-visible headers.
- Revisit the extra cache-key dimensions. They conservatively reduce reuse but
  do not model HTTP cache matching directly.
- Support cacheable response statuses and cached redirects according to HTTP
  caching rules; storage is currently limited to successful `GET`/`HEAD`
  responses.

Freshness uses explicit `Cache-Control`, `Date`, `Expires`, and `Age` metadata
where available. If no explicit lifetime is present, a valid `Last-Modified`
date supplies a conservative one-tenth apparent-age heuristic.

### 3. Finish conditional revalidation

Basic validator-aware revalidation now retains and sends both `ETag` and
`Last-Modified` when available. A `304` refreshes the stored `Vary` request
snapshot and removes variants made obsolete by a changed `Vary` field set.
Remaining work is to:

1. apply the complete HTTP freshening rules when merging `304` headers;
2. process `Set-Cookie` and other response-side effects from the validation
   response; and
3. cover validation failure, missing validators, changed validators, and
   concurrent revalidation.

`CacheMode::NoCache` now uses this path, but its request-header interaction is
not yet fully Fetch-compatible.

### 4. Finish cache invalidation

Unsafe methods invalidate every cached representation for the request URL in
the active cache partition, including method and `Vary` variants. A successful
2xx or 3xx response also invalidates a same-origin `Location` target. URL
fragments are ignored, while unrelated URLs, origins, and partitions remain
untouched.

Remaining work is to validate this behavior against the complete HTTP
invalidation rules, including status classes, redirect targets, and cache
partition boundaries.

### 5. Finish all Fetch cache modes

Complete the behavior selected by each cache mode:

- `Default`: use the normal freshness, matching, and revalidation algorithm;
- `NoStore`: bypass lookup and do not store the response;
- `Reload`: bypass an existing entry, send the required reload directives,
  and allow the refreshed response to replace the entry;
- `NoCache`: revalidate matching entries before use;
- `ForceCache`: use a matching stored response, including permitted stale
  behavior, while respecting revalidation restrictions;
- `OnlyIfCached`: enforce the same-origin restriction and return a Fetch
  network error without network access on a miss.

The current implementation has all six modes represented, but several details
remain incorrect:

- `Default` does not implement stale-while-revalidate and always waits for a
  stale response's network update.
- `Reload` adds `Cache-Control: no-cache` but omits `Pragma: no-cache` and does
  not fully honor the standard's caller-header conditions.
- `NoStore`/`Reload` do not cover all standard request-directive interactions.
- A request with caller-supplied validators does not force default mode to
  `no-store`.
- `ForceCache` and `OnlyIfCached` must use any matching response regardless of
  staleness. The implementation currently rejects some stale entries marked
  `no-cache` or `must-revalidate`.
- Cached redirects are not stored or followed for `OnlyIfCached`.
- The Rust error boundary should be aligned with the JavaScript-facing
  `TypeError` required when `only-if-cached` is used outside same-origin mode.

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

The freshness parser remains intentionally small. Add support and focused
behavior for `stale-while-revalidate`, `stale-if-error`, `s-maxage`,
`private`/`public`, `immutable`, and more complete heuristic freshness rules.
Document how these directives interact with `ForceCache`, stale responses,
revalidation failures, and partitioned browser caches.

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

- `force-cache` and `only-if-cached` with stale, `no-cache`, and
  `must-revalidate` entries;
- stale-while-revalidate behavior;
- `Pragma`, caller-supplied cache directives, and caller-supplied validators;
- validation failure, missing validators, changed validators, concurrent
  revalidation, and `Set-Cookie` during `304`;
- generated-header-aware `Vary` matching;
- cacheable non-2xx responses and cached redirects; and
- missing browser cache partitions and the resulting `no-store` behavior.

Run the focused `net` tests together with formatting and clippy before the
cache work is considered complete.
