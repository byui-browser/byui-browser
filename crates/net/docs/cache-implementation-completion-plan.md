# Net Cache Implementation Completion Plan

## Current state

The `net` crate has a working process-local cache foundation. It currently
provides:

- structured cache identity for partition, method, normalized URL, credential
  eligibility, request mode, redirect taint, and authorization presence;
- bounded storage with primary-entry and aggregate-body-byte limits;
- least-recently-used eviction and stale-entry pruning;
- multiple `Vary` request-header variants, including `Vary: *` rejection and
  variant replacement when the `Vary` field set changes;
- explicit freshness based on `Date`, `Expires`, `Age`, and supported
  `Cache-Control` directives, plus conservative `Last-Modified` heuristic
  freshness when explicit metadata is absent;
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

### 2. Complete cache identity and matching (complete)

The cache matches the browser-provided partition, method, normalized URL,
credential eligibility, request mode, redirect taint, authorization presence,
and `Vary` request-header variants. Authorization values are never retained:
authorized requests remain conservatively excluded until browser authorization
policy can support them. Response filtering is re-run when a stored response is
served, so the cache does not retain an already tainted exposed response.

Freshness uses explicit `Cache-Control`, `Date`, `Expires`, and `Age` metadata
where available. If no explicit lifetime is present, a valid `Last-Modified`
date supplies a conservative one-tenth apparent-age heuristic.

### 3. Add conditional revalidation

Implement validator-aware revalidation using response validators such as
`ETag` and `Last-Modified`. This requires:

1. retaining the applicable validator metadata with a stored response;
2. creating conditional network requests when an entry is stale or the
   request uses `no-cache`;
3. handling a `304 Not Modified` response; and
4. merging the `304` metadata with the stored representation before exposing
   the result and updating freshness.

`CacheMode::NoCache` should stop returning an unsupported-feature error once
this path is available.

### 4. Implement cache invalidation

Unsafe methods such as `POST`, `PUT`, `PATCH`, and `DELETE` must invalidate
affected cached representations using the correct partition, origin, and URL
scope. Invalidation must also work across matching redirect targets where the
Fetch algorithm requires it, without clearing unrelated partitions or
variants.

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

The current implementation treats these mostly as local lookup switches:
`NoCache` is unsupported, `Reload` lacks the required request directives,
`ForceCache` can serve stale entries without full stored-response matching,
and `OnlyIfCached` lacks its same-origin restriction.

### 6. Decide body-capture semantics

The current cache stores a response only after the caller fully consumes its
body. Confirm whether this product decision is intentional. If the cache is to
follow the standard capture behavior more closely, add bounded streaming
capture with correct handling for disturbed, failed, aborted, dropped, and
oversized bodies.

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

Run the focused `net` tests together with formatting and clippy before the
cache work is considered complete.
