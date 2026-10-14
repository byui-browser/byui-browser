# Plan: Make `net` the reusable Fetch engine

Status: implementation in progress as of 2026-10-10; steps 1 and 2 are complete
and step 3 has its shared-body foundation in place for the explicitly bounded
single-exchange HTTP(S) subset described below.

## Goal

Evolve `crates/net` from a Fetch-shaped HTTP client into the browser's reusable
Fetch engine. The crate must provide the request, header, response, body,
redirect, CORS, credential, cache, and cancellation semantics needed by any
browser consumer, including consumers that do not go through `webapis`.

`webapis` will remain responsible for JavaScript-facing bindings: WebIDL
conversion, JavaScript object identity, promises, realms, exceptions, and
JavaScript streams. This plan intentionally does not turn `net` into a JS or
DOM crate.

The target boundary is:

```text
browser / renderer / webapis
              |
              v
        net::RequestController
              |
              v
  Fetch algorithms + policy + cache + credentials
              |
              v
       transport and connection pooling
```

## Ownership boundary

`net` owns the reusable, transport-independent Fetch algorithms and the
process-local state required to execute them. In particular, `net` is
responsible for:

- Fetch-shaped `Request`, `Headers`, `Response`, and body types;
- request validation, URL/origin calculations, referrer handling, and request
  tainting;
- response types and filtering, including basic, CORS, opaque,
  opaque-redirect, and network-error responses;
- redirect processing and per-hop Fetch checks;
- body consumption, cloning, cancellation, and null-body rules;
- coordinating cache, cookie, policy, and transport services through typed
  interfaces;
- transport-independent scheduling and connection-pool admission; and
- conversion between the internal Fetch representation and the public engine
  representation.

`net` does not own browser-wide policy or durable browser state. Those remain
in other crates or browser services and are supplied to `net` through typed
traits, immutable decision objects, or request-scoped hooks:

- `webapis` owns WebIDL conversion, JavaScript object identity, promises,
  realms, exceptions, and JavaScript streams;
- `storage` owns durable cookies, HTTP-cache persistence, quotas, and storage
  partition state;
- `security` owns cookie policy, public-suffix data, CSP, mixed-content,
  CORP, MIME/nosniff, authentication, and certificate decisions;
- service-worker/browser crates own service-worker interception and lifecycle;
- renderer/browser crates own client, navigation, destination, and process
  context; and
- `devtools` and timing consumers own observable network diagnostics and
  reporting.

The `net` crate may provide default in-memory implementations for tests and
small embedders, but those implementations are not the browser's durable or
authoritative stores. A feature that requires an unavailable external owner
must be rejected with a typed error or fail closed where required by security;
it must not be represented by an inert public request field.

The ownership boundary is about behavior, not necessarily crate count. The
initial implementation may keep Fetch algorithms and private adapters in one
crate, while storage, security, and browser integrations remain replaceable
behind narrow interfaces.

## Current state and constraints

The current crate already has useful foundations:

- [`Request`](../src/api/request/definition.rs) stores Fetch-oriented request state, including
  mode, credentials, cache mode, redirect mode, referrer, abort signal, body,
  URL-list state, and priority.
- [`Headers`](../src/api/headers.rs) preserves ordered duplicate headers and has
  request, no-CORS request, response, immutable, and unrestricted guards.
- [`Body`](../src/api/body/mod.rs) is shared by requests and responses. It supports
  replayable bytes, text, URL-encoded data, blob-like bytes, one-shot bytes,
  and one-shot streams; live response bodies retain the scheduler permit.
- [`RequestController`](../src/engine/controller.rs) coordinates policy validation,
  cache lookup, cookies, scheduling, transport, response validation, and cache
  capture.
- [`ReqwestTransport`](../src/transport/reqwest.rs) currently enables native
  gzip, Brotli, deflate, and zstd response decompression. The transport owns
  content decoding, while the Fetch engine owns decoded-body limits, header
  semantics, cache representation, and failure handling.

The main limitations are documented in
[`net-fetch-standard-gaps.md`](net-fetch-standard-gaps.md), especially the
missing Fetch API behavior in item 11 and the related header, body, CORS,
credential, redirect, cache, and lifecycle gaps.

The implementation must preserve these project constraints:

- renderer processes do not open sockets directly;
- security-sensitive filtering happens before exposing response data;
- transport-specific types remain below the public Fetch boundary;
- public Rust APIs receive documentation for their purpose, ownership, units,
  and caller-visible behavior where applicable;
- no unsafe code is introduced;
- existing callers should be migrated incrementally rather than broken in one
  large change.

## Dependency strategy

Use mature crates for protocol primitives and standards data, while keeping
browser-specific Fetch behavior in `net`. A dependency is a good fit when it
provides a narrowly scoped parser, codec, storage model, or middleware seam.
It is not a substitute for request tainting, response filtering, redirect
semantics, body ownership, credentials mode, or browser policy ordering.

The current transport already uses `reqwest`, `http`, `url`, `bytes`,
`futures-util`, and Tokio. Continue to keep Reqwest below the public Fetch
boundary. Its redirect policy should remain disabled so the controller owns
Fetch redirect processing.

### Cookie storage: `cookie_store`

Use [`cookie_store`](https://docs.rs/cookie_store/latest/cookie_store/) in the
browser-owned storage/security provider when implementing the cookie hooks in
section 8. It supplies RFC 6265 cookie storage, request matching, expiration,
and public-suffix-aware behavior. The provider integration should:

1. receive the request URL, origin, credentials mode, and network partition
   key from `FetchServices`;
2. use `CookieStore::get_request_values` to construct the internal `Cookie`
   header after browser policy approves credentials;
3. pass each raw `Set-Cookie` value to `CookieStore::parse` or the equivalent
   insertion API only after response policy and CORS checks complete; and
4. persist or evict the store through `storage`, rather than making
   `cookie_store` state authoritative inside `net`.

`net` must still own the Fetch decision about whether the provider is called.
Partitioning, SameSite policy, secure-context checks, HttpOnly behavior,
storage quotas, and browser cookie permissions remain provider responsibilities.
Add an adapter test suite for domain/path matching, expiration/deletion,
Secure cookies, public suffixes, partition isolation, and credentials modes.

### HTTP cache: `http-cache-reqwest`

Evaluate [`http-cache-reqwest`](https://docs.rs/http-cache-reqwest/latest/http_cache_reqwest/)
before expanding `src/cache/memory.rs` into a complete HTTP cache. Its
middleware provides HTTP cache policy, cache modes, freshness, validators, and
pluggable cache managers. It can be integrated behind a private transport or
cache adapter, likely together with
[`reqwest-middleware`](https://docs.rs/reqwest-middleware/latest/reqwest_middleware/).

The adapter must not bypass the Fetch pipeline. It must translate between the
engine's internal request/response representation and the middleware types,
preserve network partition and origin context, keep response bodies cloneable,
and re-create a fresh filtered response for each caller. Verify Reqwest
version compatibility before adoption because middleware releases may target
a different Reqwest major version than this workspace.

Retain the custom cache layer when the required behavior is outside generic
HTTP caching, including opaque-response isolation, Fetch `OnlyIfCached`
semantics, provider authorization on cache hits, scheduler-permit ownership,
and browser-specific partitioning. The adoption decision should be recorded
with benchmarks and conformance tests for `Vary`, validators, 304 merging,
unsafe-method invalidation, and all Fetch cache modes.

### Body text decoding: `encoding_rs`

Use [`encoding_rs`](https://docs.rs/encoding_rs/latest/encoding_rs/) when
implementing `Response::text()` and any content-type/charset-aware body
decoding in section 3. It implements the Web-compatible Encoding Standard and
supports streaming decoding. The body state machine remains custom: decoding
must consume the shared body exactly once, preserve abort and transport
errors, and distinguish decoding failures from JSON or form-data parsing.

Add tests for UTF-8, declared legacy encodings, invalid byte sequences,
charset labels, BOM handling, and decoding after a partial stream read.

### Media types: `mime`

Use [`mime`](https://docs.rs/mime/latest/mime/) for parsing and normalizing
media types in `Content-Type` and body metadata. Do not use it as the complete
Fetch CORS-safelisting algorithm: the 128-byte/value restrictions and
Fetch-specific safelist rules still belong in `Headers` and the policy
modules. Add an adapter that converts parser errors into the crate's typed
request or body errors without exposing the dependency's type publicly.

### Subresource Integrity: `sha2` and `base64`

When section 11 adds SRI, use
[`sha2`](https://docs.rs/sha2/latest/sha2/) for SHA-256/SHA-384/SHA-512
digests and [`base64`](https://docs.rs/base64/latest/base64/) for integrity
metadata decoding. Implement SRI policy in `net` or the security provider,
but keep the digest primitives in these crates. Hash the response stream as it
is consumed or tee it into a verification sink; do not buffer a second copy
unless the selected body/cache design requires it. Verification must complete
before a response becomes publicly readable or cacheable, and failures must
prevent cache insertion.

Add tests for supported algorithms, malformed metadata, multiple candidates,
base64 padding, digest mismatch, successful streaming verification, aborts,
and cache behavior after verification failure.

### Scheduling and middleware: Tower and Reqwest middleware

[`tower::limit::ConcurrencyLimit`](https://docs.rs/tower/latest/tower/limit/concurrency/index.html)
can replace the admission semaphore if the transport is exposed as a Tower
`Service`. [`reqwest-middleware`](https://docs.rs/reqwest-middleware/latest/reqwest_middleware/)
can provide tracing, diagnostics, or carefully scoped middleware hooks.

Do not adopt either as a direct replacement for the current scheduler without
first proving body-lifetime semantics. Generic concurrency middleware normally
holds a permit until the response future completes, whereas this engine keeps
its permit attached to `Body` until the body is consumed or dropped.
If Tower is adopted, introduce a custom response-body guard layer that moves
the permit into the body and test cancellation, body errors, body drops, and
cache capture. Do not use generic retry middleware for requests with one-shot
bodies unless the retry layer understands replayability and Fetch error timing.

### What remains custom

Do not replace these with general-purpose crates:

- `Headers` guards and forbidden-header filtering;
- CORS checks, response tainting, and opaque/basic response views;
- controller-owned redirects and per-hop policy checks;
- shared body used/locked/disturbed state and stream teeing;
- Fetch error categories and recovery/body-consumption semantics;
- browser service hooks, credentials mode, network partition context, and
  security-policy ordering.

These are the reusable Fetch engine's core contract, not generic HTTP client
plumbing.

## 1. Establish the public contract

### 1.1 Define the engine-level API

Add a short crate-level contract to `src/lib.rs` and a design note or ADR
covering these decisions:

- `net` exposes a Rust Fetch engine, not JavaScript bindings.
- `RequestController::fetch` returns a Fetch-semantic `Response`.
- `RequestController::fetch` returns a documented `Response` with a shared
  body that can be streamed after response filtering.
- `reqwest::HeaderMap`, `reqwest::StatusCode`, `reqwest::Response`, and
  transport stream types are not part of the Fetch-facing API.
- `webapis` adapts the engine types into JavaScript `Headers`, `Request`,
  `Response`, Body methods, promises, and streams.
- Unsupported features are represented by explicit capability boundaries or
  typed errors rather than inert public fields.
- Integration points for storage, security, service workers, browser context,
  and diagnostics are traits or immutable decisions owned by those consumers;
  `net` does not define their durable state or policy databases.

### 1.2 Separate internal and exposed response data

Introduce an internal response representation containing all transport and
policy data, then construct an exposed response view from it. The internal
representation must be able to retain:

- status and status text;
- the complete internal header list, including headers not exposed to callers;
- the complete URL list and final URL;
- redirect state;
- response origin and request tainting information;
- body/null-body state;
- cache metadata;
- cookie-processing metadata.

The public response must expose only the fields allowed by its response type.
This prevents raw transport headers from bypassing CORS and filtered-response
rules.

### 1.3 Define error categories

Extend [`RequestError`](../src/api/error.rs) or introduce a dedicated Fetch error
type for:

- network errors;
- aborted operations, including abort reasons if supported;
- invalid request construction;
- invalid response construction;
- body already used, disturbed, locked, or unavailable;
- failed body decoding;
- failed CORS or same-origin checks;
- redirect-mode failures;
- unsupported body or URL schemes;
- cache-only misses represented as Fetch network errors at the public boundary.

Each public error must document whether it is recoverable, whether it occurs
before or after response headers, and whether it consumes the request or
response body.

### Step 1 completion scope

The first pass added the [engine contract](fetch-engine-contract.md), a private
[`InternalResponse`](../src/api/response/view.rs), filtered public response views, and
Fetch-oriented [`RequestError`](../src/api/error.rs) categories. `fetch` crosses
the response-exposure boundary. Public responses
use a numeric status and guarded headers rather than raw Reqwest status and
header maps. Cache-only misses now return a Fetch network error. The follow-up
work below closes 1.1–1.3 for the supported single-exchange subset.

Step 1 completion record:

- [x] **Enforce the supported contract (1.1).** Client-context requests require
      browser-owned services. Navigation, keepalive lifetime, preflight,
      integrity, priority scheduling, cache revalidation, and unsupported
      redirect modes return typed capability errors. Reqwest does not follow
      redirects; redirect statuses fail before public exposure. The supported
      process-local cache modes and their limits are stated in the contract.
- [x] **Define replaceable provider inputs (1.1).** `FetchServices` supplies
      request/response policy decisions, credential selection and storage,
      service-worker network decisions, cache partitions, and a response
      diagnostics hook. `with_services` installs it; `new` rejects requests
      carrying client context. Durable stores and policy databases stay outside
      `net`.
- [x] **Finish the public type boundary (1.1).** Public request APIs use
      `http` and `url` types directly. Controller construction and transport
      failures use engine error variants with diagnostic strings. Public
      responses use numeric status and guarded headers; Reqwest responses,
      header maps, status codes, errors, and transport streams stay private.
- [x] **Populate internal response metadata (1.2).** The supported exchange
      retains its complete one-URL list and explicit zero redirect count,
      request/response origins, request mode and response type, null-body state,
      cache provenance, and actual provider cookie-processing state. Cache hits
      retain that state. Status text is explicitly the canonical phrase; a
      custom wire phrase is not retained. Multi-hop state waits for step 7,
      with redirects rejected until then.
- [x] **Close public exposure paths (1.2).** Buffered and streaming fetches
      share `InternalResponse::expose`. Cache hits are rechecked, null and opaque
      bodies yield no bytes, and redirects fail before public headers. An
      opaque view drops its internal stream. `OpaqueRedirect` and `Error` are
      reserved variants outside this supported slice.
- [x] **Finish the supported error contract (1.3).** `RequestError` documents
      retryability, timing, and body effects by category. Redirect failure is
      emitted; response-construction and body-consumption errors are deferred
      until those operations exist. Initialization and transport failures use
      engine variants. Abort reasons are not yet supported.

The `webapis` ownership decision belongs in step 1; implementing JavaScript
`Headers`, `Request`, `Response`, promises, Body methods, and streams is later
binding work. Its current binding returns a string, so the contract must not
describe that adaptation as already complete. Likewise, the shared Body model,
complete CORS/preflight algorithms, controller-owned redirects, and durable
cache/credential integrations remain in their later numbered steps. Step 1
must provide honest capability boundaries for those features now.

## 2. Implement `Headers`

### 2.1 Complete the engine header model

Use the current [`Headers`](../src/api/headers.rs) storage as the foundation,
but make its Fetch behavior complete:

- validate header names and values at construction and mutation time;
- normalize names to ASCII lowercase for comparison and exposure;
- preserve insertion order and duplicate values;
- implement Fetch-style combination rules for `get` and iteration;
- distinguish caller mutation from internal user-agent mutation;
- make guard transitions monotonic where required;
- reject mutations on immutable lists with a stable error;
- retain a private unrestricted/internal mode for transport and policy code.

### 2.2 Add the public `Headers` API

The public `Headers` type should provide documented methods for:

- `new`;
- construction from an iterator or another header map;
- `append`;
- `set`;
- `delete`;
- `get`;
- `get_set_cookie` or an equivalent dedicated cookie accessor;
- `has`;
- ordered iteration;
- guard inspection where useful to Rust callers.

The API must not expose `reqwest::HeaderMap` directly.

### 2.3 Model header channels

Maintain distinct channels for:

- caller-controlled request headers;
- user-agent-controlled request headers such as `Origin` and `Referer`;
- internal cookie headers;
- internal response headers;
- publicly exposed response headers;
- `Set-Cookie` storage and processing.

This avoids allowing callers to set or read headers that Fetch reserves for the
user agent or cookie subsystem.

### 2.4 Add header exposure filtering

Implement response filtering for:

- basic same-origin responses;
- CORS responses and the CORS-exposed header list;
- opaque responses with no readable status, URL, headers, or body metadata;
- opaque-redirect responses;
- network-error responses;
- forbidden response headers, especially `Set-Cookie` and `Set-Cookie2`.

The filter must be applied before constructing the public `Response`, not only
when a caller queries a header.

### Step 2 completion record (2026-10-10)

The shared header model and its exposure primitives are complete for the
current engine boundary:

- [x] Added the public `Headers` type and migrated request, response, and
      trusted service metadata to use it.
- [x] Added documented `set`, `delete`, `get_set_cookie`, and iterator-based
      construction support while retaining `append` and `insert` compatibility.
- [x] Added combined ordinary-header lookup and separate `Set-Cookie` values.
- [x] Added immutable-header mutation errors and retained guard-based request,
      no-CORS, response, and immutable filtering.
- [x] Added tests for duplicate combination/order, cookie preservation, set and
      delete behavior, immutable mutation, and no-CORS filtering.
- [x] Added an internal transport-header channel so generated `Origin`, body
      metadata, `Referer`, and cookies are not written into caller-owned request
      headers.

Step 2 owns the header representation, mutation rules, and exposure primitives.
Later steps still own the algorithms that create redirect and network-error
responses; that does not leave unfinished header-layer work in this step.

### Completed header model and API work

- [x] Finished Fetch iterator semantics for ordinary fields: combine duplicate
      values according to Fetch field-combination rules while preserving the
      ordered duplicate representation needed by internal consumers.
- [x] Kept `Set-Cookie` and other non-combinable fields separate from ordinary
      combined lookup and iteration. Preserve a dedicated `get_set_cookie`
      path without joining cookie attributes with commas.
- [x] Added constructor paths for string name/value pairs and conversion from
      `HeaderMap` or another `Headers` value. Invalid names and values convert
      into the dedicated `InvalidHeaderName` and `InvalidHeaderValue` errors.
- [x] Defined and enforced case normalization, duplicate handling, and ordering
      consistently across constructors, `append`, `set`, `delete`, `get`, and
      iteration.
- [x] Made guard transitions monotonic. Stricter-to-looser transitions are
      rejected, and every attempted mutation or transition after `Immutable`
      returns a stable typed error rather than being silently ignored.
- [x] Preserved the separate caller-controlled, user-agent-controlled,
      internal-cookie, internal-response, and publicly exposed header channels
      through request preparation and transport conversion.

### Completed response exposure primitives

- [x] Completed forbidden response-header filtering, including `Set-Cookie`
      and `Set-Cookie2`, before public response construction.
- [x] Completed CORS-exposed-header parsing and filtering for safelisted names,
      explicit exposure lists, wildcard exposure with and without credentials,
      malformed exposure metadata, duplicate exposure metadata, and case-
      insensitive names.
- [x] Kept the header-layer support for opaque, opaque-redirect, and
      network-error views testable without claiming that Step 2 creates those
      response types. Actual opaque-redirect creation belongs to Step 7, and
      network-error construction remains part of the later response/CORS work.

### Completed verification and integration

- [x] Added unit tests for string/name-value conversion, invalid names and
      values, combined ordinary fields, non-combinable cookie fields, ordering,
      all guard transitions, and malformed CORS exposure metadata.
- [x] Added controller-level tests proving that caller headers remain distinct
      from generated `Origin`, `Referer`, body metadata, cookies, internal
      response headers, and publicly exposed response headers.
- [x] Verified that trusted `FetchServices` receives the complete internal
      header representation while ordinary callers receive only the filtered
      representation.
- [x] Ran `cargo fmt --all`, `cargo test --workspace`, and
      `cargo clippy --all-targets -- -D warnings`. The contract records the
      verified behavior and the later algorithms that remain explicitly
      deferred.

## 3. Implement the shared Body model

### 3.1 Introduce a common body state machine

The shared engine-level body abstraction tracks:

- body presence and null-body status;
- known byte length, when available;
- content type;
- replayability;
- whether the body is locked;
- whether it is disturbed or used;
- whether it has completed successfully;
- whether it failed, was aborted, or was canceled;
- ownership of the underlying stream and scheduler permit.

The state transitions must be explicit and tested. In particular, reading the
body through any consumption method must make later consumption fail with a
documented body-used error. Direct stream polling, helper-based consumption,
transport handoff, cache capture, body extraction, cancellation, failure, and
dropping must all participate in the same transition matrix. Invalid reads
after locking, completion, failure, abortion, or cancellation must be rejected
or terminate according to the documented state.

### 3.2 Support request BodyInit forms

Extend [`Body`](../src/api/body/mod.rs) constructors to cover the
engine-level equivalents of:

- strings and UTF-8 text;
- URL-encoded form data;
- multipart `FormData`;
- `Blob`-like bytes with a media type;
- `ArrayBuffer`/typed-array-like byte sequences;
- replayable byte buffers;
- one-shot streams.

Define an engine-owned representation for multipart form data that does not
depend on JavaScript values. `webapis` can convert JavaScript `FormData`,
`Blob`, and typed arrays into this representation.

Add `duplex` or an equivalent streaming-body capability flag where required by
the transport and redirect algorithms.

### 3.3 Add body consumption operations

Provide asynchronous engine methods corresponding to:

- `bytes`;
- `array_buffer`;
- `text` with Fetch-compatible decoding behavior;
- `json` with parsing errors separated from transport errors;
- `blob` or typed byte payload with media type;
- `form_data` for supported content types.

All methods must share the same consumption state and must preserve abort and
transport errors. `json` and `form_data` parsing failures must remain
distinguishable from transport, abort, and body-state failures. `text` must
use Fetch-compatible charset, BOM, and malformed-byte handling rather than a
hard-coded lossy UTF-8 conversion.

### 3.4 Implement cloning

Add body cloning rules:

- replayable byte bodies can be cloned without consuming the source;
- buffered response bodies can be cloned by sharing or copying immutable bytes;
- streaming bodies use a controlled tee/duplication mechanism where supported;
- disturbed, locked, failed, or already-consumed bodies cannot be cloned;
- cloning must preserve content type and null-body state.

Provide a fallible body-clone operation or equivalent validation path so clone
attempts can reject invalid states. A one-shot stream clone must not merely
share an already-consumed source when independent branches are required.
Ensure cloned streams retain correct scheduler-permit and cancellation
ownership without allowing the same network operation to be counted twice.

### 3.5 Handle null-body responses

Explicitly create null bodies for:

- `HEAD` responses;
- `CONNECT` responses where Fetch requires a tunnel/no-body result;
- status 101 where applicable to the chosen transport boundary;
- status 204;
- status 205;
- status 304;
- filtered and network-error responses.

The shared body exposed by both buffered and streaming responses must retain
the response media type, including when it is reconstructed from transport
headers, cache entries, or a buffered response. Nullness must be represented
by the body model rather than only by a separate response-side flag.

Add tests proving that null-body responses cannot accidentally expose bytes
from the transport stream.

### Step 3 completion record and remaining work

Completed in the current slice:

- [x] Added the shared public `Body` and `Blob` types in
      `src/api/body/mod.rs`. A body records nullness, known length, optional media
      type, replayability, used/completed/failed/aborted state, its one-shot
      source, and the scheduler permit retained by a live response stream.
- [x] Removed `RequestBody`, `ResponseBody`, and `StreamingResponse`.
      `Request` and `Response` now use `Body` directly, so all callers share
      the same body state and consumption API.
- [x] Preserved replayable bytes, text, URL-encoded bodies, custom media
      types, one-shot bytes, and one-shot streams. Added blob-like byte
      construction with metadata and body consumption helpers for bytes,
      UTF-8 text, and blobs.
- [x] Made replayable-body clones independent and one-shot-body clones share
      consumption state. A second helper-based read now returns the typed
      `BodyAlreadyUsed` error instead of silently observing a second source.
- [x] Moved cancellation wrapping, scheduler-permit attachment, cache capture,
      and response streaming onto the shared body implementation.
- [x] Replaced exposure-time empty streams with explicit null bodies for
      filtered views and all currently detected null-body status cases.
- [x] Added shared-body regression tests for replayable and one-shot clone
      behavior, body-used errors, blob metadata, and stream failure handling.

The following remains before Step 3 is fully complete:

- [x] Exposed body state and one-shot consumption methods through public
      request and response bodies. `fetch` returns response metadata at
      headers and a `Body` that may remain live with the same API.
- [ ] Complete the body state transition contract for public and internal
      readers. Direct reads must respect locked and terminal states, internal
      transport/cache extraction must update the same state, and cancellation
      must distinguish abort, failure, cancellation, and dropping.
- [ ] Make nullness authoritative in `Body` rather than duplicating it in the
      separate `InternalResponse::body_is_null` flag. Ensure all response
      construction, filtering, caching, and buffering paths use the same null
      body representation.
- [ ] Preserve known response body length when available, including
      `Content-Length` from live transport responses and cached responses, and
      retain that metadata through body wrapping and cache capture.
- [ ] Decide whether completion, failure, abortion, and cancellation state is
      intentionally internal or needs documented public accessors. The public
      body-state contract must match the state tracked by the implementation.
- [x] Added `array_buffer`, generic `json`, and `form_data` consumption
      methods. JSON and form parsing now return typed parse errors distinct
      from transport, abort, and body-state failures.
- [x] Added charset- and BOM-aware text decoding through `encoding_rs` and
      propagated `Content-Type` metadata from live transports, cache hits, and
      cache-capture wrappers into response bodies.
- [ ] Define JSON-specific decoding semantics independently of generic text
      decoding, including BOMs, malformed UTF-8, and conflicting or non-UTF-8
      `Content-Type` charset declarations, and add focused tests.
- [x] Added engine-owned `FormData` and `FormDataEntry` types. Text-only forms
      serialize as URL-encoded bodies; file-bearing forms serialize as
      multipart bodies; and both URL-encoded and supported multipart bodies
      can be parsed through `Body::form_data`.
- [ ] Complete binary-safe multipart parsing, RFC 5987 filename handling,
      typed-array convenience inputs, and a documented duplex capability.
- [ ] Harden multipart serialization with collision-safe boundaries, escaped
      field names and filenames, CR/LF and quote validation, and adversarial
      serializer/parser round-trip tests.
- [ ] Replace shared one-shot stream cloning with a bounded controlled tee;
      define branch backpressure, dropping, cancellation, cache capture, and
      scheduler-permit ownership rules. Reject cloning of disturbed, locked,
      failed, aborted, canceled, or already-consumed bodies.
- [ ] Define the relationship between the infallible Rust `Clone` implementation
      and Fetch's fallible body-clone rules. Remove, restrict, or clearly mark
      compatibility cloning where invalid body states must be rejected.
- [ ] Add a complete body-state transition matrix, including direct stream
      readers, helper consumption, transport handoff, cache extraction,
      dropping, every null-body source and response status, and terminal
      failed/aborted/canceled states.
- [ ] Add a distinct canceled state and define how cancellation differs from
      abort, transport failure, and ordinary body dropping.
- [ ] Define and test the `CONNECT` response null-body path, or explicitly
      document why the browser-facing rejection of `CONNECT` makes that Fetch
      case unreachable in this engine boundary.
- [ ] Construct explicit null-body network-error responses when the later
      response/CORS algorithms create `ResponseType::Error`, rather than
      exposing an empty but readable body or returning only a generic error.
- [ ] Add controller-level coverage for body consumption APIs, decoding,
      parsing, stream teeing, permit release, cache effects, and every
      buffered/streaming parity case, including response media types and
      network-error/null-body behavior.

## 4. Expand `Request` and scheme dispatch

### 4.1 Add a complete engine-level constructor

Extend [`Request::new`](../src/api/request/definition.rs) and add an options/init structure
that can configure:

- URL and base environment;
- method;
- `Headers`;
- BodyInit;
- mode;
- credentials;
- cache mode;
- redirect mode;
- referrer and referrer policy;
- integrity metadata;
- keepalive;
- priority;
- destination and initiator;
- service-worker mode;
- abort signal.

Apply Fetch validation during construction and option updates, including:

- method normalization;
- forbidden methods;
- no-CORS method restrictions;
- body restrictions for `GET` and `HEAD`;
- no-CORS header and content-type restrictions;
- body metadata insertion;
- keepalive byte quotas;
- URL parsing, base-URL resolution, and scheme validation;
- dispatch to the supported scheme-specific fetch algorithm.

### 4.2 Add Fetch-visible accessors and lifecycle methods

Document and expose accessors for all supported request properties, plus:

- `body_used`;
- body consumption helpers;
- `clone`;
- body stream access for Rust consumers;
- resolved/current URL where appropriate;
- redirect URL-list state for internal consumers without exposing forbidden
  information to filtered callers.

Keep internal redirect bookkeeping separate from the original request URL and
from any future JS-facing URL serialization.

### 4.3 Preserve internal request preparation

Refactor [`Request::fetch_headers`](../src/api/request/definition.rs), cookie
attachment, referrer calculation, and origin calculation so they operate on
the internal request/header channels.
Caller-visible headers must not gain the internally generated `Origin`,
`Referer`, or cookie values unless the Fetch contract permits them.

### 4.4 Add scheme-specific fetch dispatch

Add a controller-level scheme dispatcher so request construction and execution
do not assume that every valid URL is an HTTP(S) transport request. The
dispatcher must:

- retain HTTP(S) as the existing network path;
- implement `data:` fetches, including percent- and base64-encoded payloads,
  media-type/default-type handling, decoded bytes, and invalid-payload errors;
- implement `blob:` resolution through an explicit browser-owned object-URL
  or blob provider, without making `net` own the blob registry;
- implement `file:` access only through an explicit filesystem/security
  provider, with local-file authorization, origin assignment, and failure
  behavior defined by that provider;
- implement the browser-specific `byui:` scheme for local browser pages,
  routing registered page paths through a browser-owned page provider rather
  than the filesystem or HTTP transport. The provider must define page
  lookup, response metadata, origin assignment, and access-control behavior;
- implement the supported `about:` fetch cases, including the required
  empty/blank response behavior, and reject unsupported `about:` URLs;
- assign the resulting response URL, origin, status, headers, body, and
  null-body state through the same internal response and public exposure
  pipeline used by HTTP(S);
- reject unsupported schemes with the typed URL/scheme error before invoking
  the HTTP transport or any unrelated provider.

Define the ownership boundary for filesystem, blob, local-page, and
`byui:` page data in `FetchServices` or dedicated provider traits. Do not add
durable file access, blob registries, browser-origin policy databases, or
browser page registries to `net`. Add focused tests for each supported scheme,
including `byui:` page routing, malformed and unsupported inputs, origin
assignment, credential behavior, response filtering, and body/null-body
semantics.

## 5. Expand `Response`

### 5.1 Add response metadata

Replace the current raw fields in [`Response`](../src/api/response/types.rs) with a
Fetch-oriented representation containing:

- response type;
- status code;
- status text/reason phrase where supported;
- exposed headers;
- exposed URL;
- redirected flag;
- body;
- cache provenance if retained as a Rust diagnostic field.

Add derived helpers such as `ok` and explicit methods for checking whether the
response has a body.

### 5.2 Add response constructors

Provide documented constructors for engine consumers where appropriate:

- ordinary responses from internal network results;
- network-error responses;
- empty/null-body responses;
- redirects with validated status and location;
- byte/text/JSON responses for browser-internal producers.

Construction must validate status, header guards, body/status combinations, and
URL exposure rules.

### 5.3 Add response cloning and body methods

Implement `Response::clone`, `body_used`, body stream access, and all supported
body consumption methods through the shared Body model.

`fetch` should return a response whose body is consumable exactly once, while
`clone` produces an independently consumable response according to the body
cloning rules.

### 5.4 Unify response delivery

Completed: `StreamingResponse` and `fetch_stream` were removed. `fetch`
returns `Response` at header availability, and `Response.body` is the sole
live or buffered body channel. Filtering, null-body handling, cancellation,
cache capture, and scheduler-permit ownership now follow that single path.

Update the scheduler-permit handling currently implemented by
[`Body::into_parts`](../src/api/body/mod.rs) and
[`Body::attach_permit`](../src/api/body/mod.rs) so it survives body
cloning, teeing, cancellation, and cache capture.

## 6. Implement Fetch response filtering and CORS integration

### 6.1 Complete request tainting and response types

Extend the current CORS and policy modules to determine:

- same-origin versus cross-origin requests;
- request mode and credentials interaction;
- basic, CORS, opaque, opaque-redirect, and error response types;
- exposed status, URL, headers, and body for each type.

The controller must never return the raw transport response for a response
type that requires filtering.

### 6.2 Implement CORS checks

Complete the controller path around the currently permissive
[`CorsChecker::response_type`](../src/policy/cors.rs) by adding:

- simple-request checks;
- preflight `OPTIONS` requests;
- `Access-Control-Allow-Origin` validation;
- credentials validation;
- allowed-method validation;
- allowed-header validation;
- exposed-header parsing;
- preflight cache;
- cross-origin redirect checks;
- network-error conversion on failure.

### 6.3 Add same-origin and opaque behavior

Enforce `SameOrigin` failures before exposing a response. For `NoCors`, return
an opaque response with hidden status, URL, headers, and body metadata while
still allowing the internal fetch to complete when the algorithm permits it.

## 7. Move redirects into the controller

Disable Reqwest's automatic redirect following for the Fetch transport.
Implement a controller-owned redirect loop that:

- validates each `Location` value;
- accepts only supported redirect schemes;
- enforces the Fetch redirect limit of 20;
- updates the request URL list and current URL;
- honors `follow`, `error`, and `manual` modes;
- rewrites methods and bodies for applicable status codes;
- rejects or stops when a one-shot body cannot be replayed;
- strips credentials and sensitive headers when crossing origins;
- re-runs origin, CORS, policy, cookie, and integrity checks per hop;
- produces an opaque-redirect result for manual mode where required.

Add redirect metadata to the internal response without exposing the complete
URL list through filtered public responses.

## 8. Complete credentials and cookie channels

`net` owns the Fetch credentials algorithm and the internal cookie-header
channel. It does not own durable cookie storage or the browser's authoritative
cookie policy. The cookie service supplied by `security`/`storage` is
responsible for matching, persistence, partitioning, and policy decisions;
`net` supplies the request URL, origin, credentials mode, and response metadata
and applies the service's typed result.

### 8.1 Implement cookie storage integration

Extend the step-1 `FetchServices` credential hooks with the full Security &
Storage cookie implementation. The typed interface already accepts Fetch
request context and raw `Set-Cookie` values without exposing storage internals
to `net`. The owning service must support:

- domain and path matching;
- expiry and deletion;
- `Secure` and `HttpOnly` rules;
- SameSite behavior;
- public-suffix checks;
- network partition keys;
- credentials mode;
- response `Set-Cookie` processing.

`net` should provide an in-memory test implementation only where useful. It
must not become the owner of persistent cookie files, storage quotas, public
suffix data, or site-wide cookie policy.

### 8.2 Keep cookie headers internal

Attach cookies through the internal request-header channel. Do not allow
callers to set `Cookie` through ordinary `Headers`, and do not expose
`Set-Cookie` through ordinary response headers.

### 8.3 Integrate credentials with CORS

Ensure `omit`, `same-origin`, and `include` alter behavior. Credentialed
cross-origin requests must require compatible CORS response headers and must
not be converted into readable responses otherwise.

## 9. Replace the TTL-only cache representation

`net` owns Fetch cache semantics and the interface used to look up, validate,
revalidate, store, and clone response representations. `storage` owns durable
cache storage, quotas, eviction, persistence, and partition-key state. A
process-local in-memory cache may remain as a default implementation for tests
and embedders, but it is not the browser's durable HTTP cache.

The response/body redesign must be reflected in `cache/mod.rs`:

- cache internal responses, not public filtered responses;
- store bodies in a cloneable representation;
- pass network partition/origin context to the cache service rather than
  defining durable partition storage in `net`;
- include request method, relevant headers, credentials, and response tainting
  in matching;
- implement `Vary` matching;
- process freshness from `Date`, `Expires`, `Age`, and `Cache-Control`;
- support validators and conditional requests;
- merge 304 responses;
- invalidate entries after unsafe methods;
- implement the Fetch cache modes rather than simple lookup switches;
- return a Fetch network error for `OnlyIfCached` misses at the public API.

Cache lookup must produce a fresh exposed `Response` view for each caller so
one caller consuming a body cannot consume the cached copy for another caller.

## 10. Complete cancellation and lifecycle behavior

Extend `AbortSignal` and controller lifecycle handling to support:

- serialized abort reasons;
- ongoing, terminated, and aborted states;
- cancellation during DNS/connection, redirects, preflight, cache work,
  cookie processing, response filtering, and body consumption;
- consistent body error state after cancellation;
- cancellation of all cloned/tee'd body branches;
- no cache insertion after an aborted or failed body.

Ensure the public API distinguishes a Fetch network error from a caller abort
when the engine contract requires that distinction.

## 11. Integrate policy and browser hooks without coupling to `webapis`

Keep the Fetch engine independent of JavaScript. `net` owns the point in the
Fetch algorithm at which each decision is required, but the crate that owns
the relevant browser policy supplies the decision through a typed hook or
immutable input. `net` must not import those crates merely to inspect their
internal state.

The integration points include:

- CSP and mixed-content checks;
- Upgrade Insecure Requests;
- service-worker interception and bypass;
- Fetch Metadata headers;
- CORP and MIME/nosniff checks;
- Subresource Integrity verification;
- keepalive aggregate quotas;
- offline/network state;
- timing and server-timing records;
- TLS client certificates and HTTP authentication entries.

Ownership of the integrations is divided as follows:

- `security` supplies CSP, mixed-content, Upgrade Insecure Requests, CORP,
  MIME/nosniff, SRI policy, TLS client-certificate, and authentication
  decisions;
- service-worker/browser crates supply interception, bypass, and lifecycle
  decisions;
- renderer/browser crates supply client, destination, initiator, navigation,
  offline, and network-state context;
- `storage` supplies keepalive quotas and partition/storage decisions where
  those decisions depend on durable browser state; and
- `devtools`/timing consumers receive timing and Server-Timing records rather
  than making the Fetch request path depend on their data model.

For each hook, document whether `net` fails closed, returns a typed
unsupported error, or proceeds with a specified default when no provider is
installed. `net` must not silently accept a security-sensitive request merely
because an integration provider is absent.

## 12. Migrate current modules and callers

### 12.1 Request and response modules

- Split transport preparation from public Fetch types.
- Move `reqwest` conversions into private transport adapters.
- Replace public `HeaderMap` fields with `Headers`.
- Replace direct `Vec<u8>` response bodies with the shared Body abstraction.
- Preserve compatibility constructors temporarily if needed, with deprecation
  documentation and migration tests.

### 12.2 Controller

Refactor `fetch` into a pipeline with explicit stages:

1. request construction and validation;
2. URL/origin checks and requests to the owning policy providers;
3. cache lookup;
4. service-worker interception hook;
5. preflight if required;
6. cookie and user-agent header preparation;
7. controller-owned redirect loop;
8. transport request;
9. transport decoding and internal response construction;
10. response policy/CORS/integrity decisions from owning providers;
11. response filtering;
12. cookie processing;
13. cache insertion;
14. public response creation.

Each stage should have focused tests and typed errors.

### 12.3 Transport

Keep `ReqwestTransport` responsible for HTTP I/O only. It should:

- receive an already-prepared internal request;
- not decide Fetch redirect behavior;
- not expose raw responses above the controller;
- preserve streaming and abort behavior;
- report status, headers, URL, and body chunks to the controller.

#### Response decompression policy

Keep Reqwest's native response decompression for gzip, Brotli, deflate, and
zstd. It already performs incremental decoding and removes `Content-Encoding`
and `Content-Length` after decoding. Make that behavior explicit in the
transport adapter rather than relying only on dependency feature defaults.

The adapter must:

- preserve an explicitly supplied `Accept-Encoding` header;
- avoid implicit content encoding for requests carrying `Range` unless the
  selected Fetch/cache policy explicitly permits it;
- keep automatically generated `Accept-Encoding` internal to transport
  preparation;
- convert decompression failures into the crate's transport/body error type;
- expose decoded bytes to the body state machine while retaining any required
  wire metadata internally for diagnostics and cache matching;
- enforce a decoded response-size limit and, where appropriate, a maximum
  decompression ratio before forwarding or caching unbounded output; and
- release the scheduler permit and prevent cache insertion after decompression
  failure, abort, or limit violation.

Add a transport configuration object for decompression limits rather than
hard-coding limits in the response type. The limit must count decoded bytes
across streaming chunks, work for both buffered and streaming fetches, and
produce a documented typed error. If the cache stores decoded bodies, include
that choice in the cache contract. If it stores encoded representations,
retain `Content-Encoding`, `Content-Length`, validators, and
`Vary: Accept-Encoding` in the internal representation and decode only for
the public body.

Do not move decompression into the Fetch policy layer unless the browser needs
raw wire bytes for a signature or specialized cache path. The public response
must describe the decoded representation, while internal response metadata may
retain wire-level information.

## 13. Test and conformance plan

### 13.1 Unit tests

Add unit tests for:

- header name/value validation;
- case-insensitive lookup;
- duplicate ordering and combination;
- all header guards;
- `Set-Cookie` hiding;
- request option validation;
- method/body restrictions;
- Body state transitions;
- body cloning and tee behavior;
- text/JSON/byte decoding;
- null-body statuses;
- response type and metadata exposure;
- abort and body error transitions;
- redirect method/body rewriting;
- URL-list bookkeeping;
- cache matching and `Vary` behavior.

### 13.2 Local integration tests

Extend the existing request/cache/redirect test servers with cases for:

- same-origin and cross-origin requests;
- simple CORS and preflighted CORS;
- credentialed and failed CORS;
- no-CORS opaque responses;
- manual and error redirects;
- one-shot body redirect failures;
- `HEAD`, 204, 205, and 304 responses;
- streamed body errors and cancellation;
- cached body cloning;
- cookie persistence and credentials modes;
- filtered response headers;
- integrity success and failure.
- gzip, Brotli, deflate, and zstd response decoding;
- malformed/truncated compressed bodies;
- decoded-size and decompression-ratio limits;
- range requests and explicit `Accept-Encoding` behavior;
- cache insertion and validator behavior after decoding.

### 13.3 Conformance tests

Select a realistic Fetch subset from Web Platform Tests after the engine-level
API stabilizes. Track unsupported tests explicitly, especially for features
owned by service workers, storage, CSP, navigation, or JavaScript bindings.

## 14. Documentation and migration work

- Update `src/lib.rs` to describe the new engine boundary.
- Update `net-fetch-standard-gaps.md` as each gap becomes behaviorally
  implemented.
- Add a public API guide showing direct Rust consumers constructing requests,
  executing fetches, filtering responses, and consuming bodies.
- Document which types are safe to send across the Network/Renderer IPC
  boundary and which remain process-local.
- Update `docs/TECH_ARCHITECTURE.md` to distinguish the `net` Fetch engine from
  the `webapis` JavaScript Fetch binding.
- Record the ownership boundary in `docs/TECH_ARCHITECTURE.md`, including which
  crate supplies cookie, cache, security, service-worker, browser-context, and
  diagnostics providers.
- Add an ADR because this changes the ownership interpretation of “Fetch”
  between the Networking and JS APIs teams.
- Document migration paths for callers using raw `HeaderMap` and the former
  buffered `fetch` body behavior.

## 15. Suggested implementation order

Implement in vertical slices so the browser retains a working network path:

1. Contract and internal/public response split.
2. `Headers` API and transport conversion.
3. Shared Body state for buffered and streaming bytes; use `encoding_rs` for
   Web-compatible text decoding and `mime` for media-type parsing once body
   consumption is introduced. Define transport decompression limits and
   decoded-versus-wire metadata at this boundary.
4. Fetch-style `Request` and `Response` accessors/cloning and scheme dispatch.
5. Null-body responses and body consumption methods.
6. Response filtering and CORS exposure.
7. Controller-owned redirects.
8. Credential/cookie channels and credentials modes; implement the browser
   provider with `cookie_store` behind `FetchServices`.
9. Cache representation and standards-aware cache behavior; evaluate
   `http-cache-reqwest` plus `reqwest-middleware` before expanding the custom
   cache, retaining an adapter for Fetch-specific partitioning and body state.
10. Cancellation/lifecycle completion.
11. Integrity, keepalive, service-worker, CSP, CORP, timing, and other policy
    hooks; use `sha2` and `base64` for SRI primitives.
12. Web Platform Tests and removal of compatibility shims.

Do not begin with a wholesale rename of every type. Stabilize the internal
response/body model and migrate callers one boundary at a time.

## 16. Acceptance criteria

The change is complete when:

- direct Rust consumers can perform Fetch-semantic requests without importing
  `webapis`;
- no public response path exposes a raw `reqwest::HeaderMap`;
- request and response bodies implement documented used/locked/disturbed,
  consumption, cloning, abort, and null-body behavior;
- response filtering is enforced before public exposure;
- redirect modes and URL-list behavior are controller-owned;
- credentials, cookies, CORS, and cache behavior are integrated rather than
  stored as inert request fields;
- native response decompression remains enabled, with documented decoded-body
  limits, range/`Accept-Encoding` behavior, internal wire metadata, and
  failure-safe cache insertion;
- `webapis` can wrap the engine without reimplementing security or networking
  semantics;
- storage, security, service-worker, browser-context, and diagnostics owners
  can provide their integrations without importing JavaScript types or taking
  ownership of `net`'s Fetch state machine;
- browser code can continue using `RequestController` directly;
- focused tests cover every newly supported behavior and every intentional
  limitation;
- `cargo fmt --all`, relevant `cargo test` commands, and
  `cargo clippy --all-targets -- -D warnings` pass.
