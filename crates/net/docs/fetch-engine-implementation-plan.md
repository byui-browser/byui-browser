# Plan: Make `net` the reusable Fetch engine

Status: proposed implementation plan as of 2026-10-09.

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

- [`Request`](../src/request.rs) stores Fetch-oriented request state, including
  mode, credentials, cache mode, redirect mode, referrer, abort signal, body,
  URL-list state, and priority.
- [`HeaderList`](../src/request.rs) preserves ordered duplicate headers and has
  request, no-CORS request, response, immutable, and unrestricted guards.
- [`RequestBody`](../src/request.rs) supports replayable bytes, text,
  URL-encoded data, one-shot bytes, and one-shot streams.
- [`ResponseBody`](../src/response.rs) provides an abort-aware asynchronous
  byte stream and holds the scheduler permit while it is live.
- [`RequestController`](../src/controller.rs) coordinates policy validation,
  cache lookup, cookies, scheduling, transport, response validation, and cache
  capture.

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

## 1. Establish the public contract

### 1.1 Define the engine-level API

Add a short crate-level contract to `src/lib.rs` and a design note or ADR
covering these decisions:

- `net` exposes a Rust Fetch engine, not JavaScript bindings.
- `RequestController::fetch` returns a Fetch-semantic `Response`.
- `RequestController::fetch_stream` either becomes an internal implementation
  detail or returns a documented streaming `Response` variant with identical
  filtering and lifecycle rules.
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

Extend [`RequestError`](../src/error.rs) or introduce a dedicated Fetch error
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

## 2. Implement `Headers`

### 2.1 Evolve `HeaderList` into the engine header model

Use the current [`HeaderList`](../src/request.rs) storage as the foundation,
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

Add a public `Headers` type or rename `HeaderList` to `Headers` while retaining
an internal alias during migration. It should provide documented methods for:

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

## 3. Implement the shared Body model

### 3.1 Introduce a common body state machine

Replace the split behavior between `RequestBody` and `ResponseBody` with a
shared engine-level body abstraction that tracks:

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
documented body-used error.

### 3.2 Support request BodyInit forms

Extend the current [`RequestBody`](../src/request.rs) constructors to cover the
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
transport errors.

### 3.4 Implement cloning

Add body cloning rules:

- replayable byte bodies can be cloned without consuming the source;
- buffered response bodies can be cloned by sharing or copying immutable bytes;
- streaming bodies use a controlled tee/duplication mechanism where supported;
- disturbed, locked, failed, or already-consumed bodies cannot be cloned;
- cloning must preserve content type and null-body state.

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

Add tests proving that null-body responses cannot accidentally expose bytes
from the transport stream.

## 4. Expand `Request`

### 4.1 Add a complete engine-level constructor

Extend [`Request::new`](../src/request.rs) and add an options/init structure
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
- URL and scheme validation.

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

Refactor `apply_fetch_headers`, cookie attachment, referrer calculation, and
origin calculation so they operate on the internal request/header channels.
Caller-visible headers must not gain the internally generated `Origin`,
`Referer`, or cookie values unless the Fetch contract permits them.

## 5. Expand `Response`

### 5.1 Add response metadata

Replace the current raw fields in [`Response`](../src/response.rs) with a
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

### 5.4 Replace or narrow `StreamingResponse`

Decide whether `StreamingResponse` remains public. The preferred direction is
to make it an internal transport/controller representation and expose a
streaming body through `Response` instead. If compatibility requires retaining
it, document it as an engine-level response view and guarantee that it applies
the same filtering, null-body, cancellation, and body-state rules as the
buffered path.

Update the scheduler-permit handling currently implemented in
[`ResponseBody::into_parts`](../src/response.rs) and
[`ResponseBody::attach_permit`](../src/response.rs) so it survives body
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
[`CorsChecker::validate`](../src/cors/validator.rs) by adding:

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

Replace the current no-op cookie operations in `cookies/jar.rs` with a typed
interface to the Security & Storage owners. The interface should accept
Fetch-relevant inputs and return headers or processing decisions without
exposing the storage implementation to `net`. The owning service must support:

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

Refactor `fetch` and `fetch_stream` into a pipeline with explicit stages:

1. request construction and validation;
2. URL/origin checks and requests to the owning policy providers;
3. cache lookup;
4. service-worker interception hook;
5. preflight if required;
6. cookie and user-agent header preparation;
7. controller-owned redirect loop;
8. transport request;
9. internal response construction;
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
- Document compatibility/deprecation paths for callers using raw `HeaderMap`,
  `Response.body: Vec<u8>`, and `StreamingResponse`.

## 15. Suggested implementation order

Implement in vertical slices so the browser retains a working network path:

1. Contract and internal/public response split.
2. `Headers` API and transport conversion.
3. Shared Body state for buffered and streaming bytes.
4. Fetch-style `Request` and `Response` accessors/cloning.
5. Null-body responses and body consumption methods.
6. Response filtering and CORS exposure.
7. Controller-owned redirects.
8. Credential/cookie channels and credentials modes.
9. Cache representation and standards-aware cache behavior.
10. Cancellation/lifecycle completion.
11. Integrity, keepalive, service-worker, CSP, CORP, timing, and other policy
    hooks.
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
