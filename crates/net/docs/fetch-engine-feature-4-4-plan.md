# Feature 4.4 plan: scheme-specific Fetch dispatch

Status: provider boundary and engine-owned `data:` handler in progress; blob,
file, byui, and about handlers remain, reviewed against the current
`crates/net` implementation on 2026-10-14.

This plan records only the work that remains for feature 4.4 of the
[`fetch-engine-implementation-plan.md`](fetch-engine-implementation-plan.md).
It is intentionally narrower than a complete Fetch implementation. Redirects,
full CORS preflight, durable cookies, standards-compliant caching, and the
remaining body-state work belong to later plan sections unless they are needed
to make a scheme dispatch decision safe and observable.

## Current implementation baseline

The following foundations are already present and should be reused:

- Step 1 has an internal response representation and a public filtered
  response boundary. `InternalResponse::expose` applies response-type,
  header, URL, status, body, and null-body filtering.
- Step 2 has ordered guarded headers and separate internal response/request
  header paths. `Set-Cookie` and internally generated request headers are not
  automatically exposed to callers.
- Step 3 has the shared `Body` type, replayable and one-shot byte sources,
  streaming bodies, media-type metadata, body consumption helpers, and explicit
  null bodies for the supported response cases.
- `Request` already stores the original URL, current URL, URL-list state,
  request mode, credentials mode, environment, referrer policy, and body.
- `Request::fetch_headers` builds an internal header channel; cookies are
  attached by the controller only when credentials allow them; `Referer` is
  added by the HTTP transport from the prepared request.
- The HTTP(S) path, scheduler, cache, response policy, and existing tests must
  remain unchanged for HTTP(S) requests.

## Completed first pass

The initial pipeline refactor is complete:

- Added the internal `FetchScheme` classification for `http`, `https`, `data`,
  `blob`, `file`, `byui`, and `about`.
- Changed request validation to return both the normalized URL and its selected
  scheme through `ValidatedUrl`.
- Kept unsupported schemes such as `ftp:` rejected before side effects.
- Added the controller dispatch match in `RequestController::fetch`.
- Isolated the existing HTTP(S) implementation in `fetch_http`.
- Restricted cache eligibility and cache partition lookup to HTTP(S).
- Added unit coverage for scheme classification and an integration test proving
  a supported non-HTTP URL reaches the dispatcher seam.

The `data:` branch now decodes percent-encoded and base64 payloads into the
shared response/body pipeline. Provider-backed branches fail with a typed
`MissingProvider` error when their matching capability is absent; installed
providers still await their scheme handlers.

## Next steps

### 1. Define the scheme and provider boundary

The URL classification and dispatcher seam are complete. Finish the boundary
work below before implementing any provider-backed scheme handler.

#### 1.1 Define provider-neutral resource results — complete

The first-pass [`SchemeResource`](../src/api/providers/body.rs) and
[`SchemeBody`](../src/api/providers/body.rs) types now provide the provider
boundary. `SchemeBody` supports explicit null bodies, materialized bytes, and
sendable `Vec<u8>` chunk streams for future non-buffered providers. `net`
converts these values into `Body` and then `InternalResponse`; public fetches
continue to return `Response`.

The resource contains only the metadata needed to construct an
`InternalResponse`:

- resolved response URL;
- body bytes or a sendable body stream;
- media type;
- response headers permitted by the provider contract;
- status and status text where applicable;
- response origin; and
- null-body state where applicable.

The type must not contain a browser registry, filesystem handle, storage
object, or browser-specific policy database. Providers return this data to
`net`; `net` remains responsible for constructing the internal and public
Fetch responses.

The first implementation uses [`SchemeResource`](../src/api/providers/body.rs) with
`SchemeBody::Null`, `SchemeBody::Bytes`, and `SchemeBody::Stream` variants.
The stream variant uses a sendable `Vec<u8>` chunk stream so browser providers
can add streaming later without changing the public `Response` return value.
`net` converts the resource into `InternalResponse` and then into the existing
public `Response`.

Alternatives considered:

- Return `Body` directly from providers. This is simpler for byte resources but
  makes future provider streaming depend on exposing more of `Body`'s internal
  construction and lifecycle API.
- Return only `Vec<u8>`. This is the smallest first implementation, but it
  forces files, blobs, and pages to buffer before the provider boundary and is
  therefore not the selected design.
- Return a public `Response` from providers. This would bypass centralized
  origin, CORS, header, credential, null-body, and response filtering logic and
  is explicitly rejected.

#### 1.2 Define narrow provider traits

Add provider traits in the `api/providers/` module alongside
[`services.rs`](../src/api/services.rs):

- `BlobProvider` resolves or rejects a `blob:` URL;
- `FileProvider` authorizes and opens a `file:` URL; and
- `PageProvider` resolves a registered `byui:` page.

Keep `data:` parsing inside `net`; it does not need a provider. The initial
`about:blank` behavior can also remain engine-owned.

Each trait should accept the request context needed for authorization without
accepting JavaScript or DOM types. Provider methods should be asynchronous if
the resource may require I/O, while allowing a small synchronous adapter for
embedded pages and tests.

#### 1.3 Decide how providers enter `FetchServices`

Choose and document one integration shape:

- optional provider accessors on [`FetchServices`](../src/api/services.rs); or
- a separate browser-owned provider bundle installed alongside
  `FetchServices`.

A separate bundle is preferred if scheme-specific capabilities would make
`FetchServices` a broad catch-all interface. In either design, the controller
must receive providers through `with_services` or an equivalent constructor;
there must be no global registry lookup.

#### 1.4 Preserve existing service implementations

New provider accessors should have default implementations that return
`UnsupportedFeature`, or providers should be optional in the bundle. This
keeps existing test and standalone service implementations source-compatible
until they opt into a scheme.

Document which missing-provider errors occur before any provider or transport
side effect and which provider failures may occur after a resource has been
opened.

#### 1.5 Define failure and security semantics

Document typed behavior for:

- provider absent;
- resource missing;
- blob revoked;
- file access denied;
- malformed provider metadata; and
- invalid or unavailable provider-assigned origins.

Providers own authorization, sandboxing, path resolution, revocation, and
durable state. `net` must fail closed when a required provider or security
decision is unavailable.

#### 1.6 Wire providers into controller state

Extend [`ControllerInner`](../src/engine/controller.rs) and
`RequestController::with_services` so the dispatcher can obtain the selected
provider without owning its registry. Keep provider references shareable and
thread-safe in the same way as the existing `Arc<dyn FetchServices>`.

The controller should also make it impossible for a `blob:` request to call a
file provider, or for a `byui:` request to fall through to HTTP transport.

#### 1.7 Add browser-side composition points

The browser currently creates a standalone controller with
`RequestController::new` in [`crates/browser/src/main.rs`](../../browser/src/main.rs).
Add the composition point for `with_services` and keep concrete implementations
outside `net`:

- `crates/browser` owns the `byui:` page provider and page registration;
- browser/storage owns the blob URL registry and revocation state; and
- browser/security or the designated filesystem owner handles authorized
  `file:` access, sandboxing, and origin policy.

The existing [`Page`](../../browser/src/lib.rs) and embedded welcome-page
resources are the starting point for the browser-owned page provider.

#### 1.8 Add provider-boundary tests

Before implementing the handlers, add tests proving that:

- absent providers return typed capability errors;
- unsupported schemes do not invoke any provider;
- each supported scheme can access only its matching provider;
- provider metadata can be converted without leaking provider state;
- browser-context composition uses `with_services`; and
- existing `FetchServices` implementations remain source-compatible.

These tests belong beside the service/dispatcher integration tests. Tests for
actual decoding, file reads, blob resolution, page lookup, and `about:` body
semantics belong to the later scheme-handler steps.

The supported scheme classification remains:

```text
Http, Https, Data, Blob, File, Byui, About
```

### 2. Replace the dispatcher placeholders with scheme handlers

The controller-level seam now exists in
[`RequestController::fetch`](../src/engine/execute.rs). Replace its non-HTTP
placeholder branches with handlers in this order:

1. `data:` parsing and response construction.
2. `blob:` provider resolution.
3. `file:` provider-authorized access.
4. `byui:` browser-page routing.
5. The explicitly supported `about:` cases.

Each handler must return an `InternalResponse`; it must not expose a new public
response type or call the HTTP transport directly.

The target common pipeline is:

```text
prepare request
  -> HTTP(S)-only cache lookup
  -> build internal request headers
  -> dispatch by scheme
  -> finalize InternalResponse
  -> run response policy and browser services
  -> expose filtered Response
```

The completed dispatcher must continue to:

- retain the current scheduler and Reqwest path for HTTP(S);
- bypass HTTP transport, HTTP cookies, and HTTP cache for local schemes unless
  a later feature explicitly defines those behaviors;
- invoke only the provider belonging to the selected scheme once handlers are
  installed;
- reject missing providers with a typed capability error;
- route every successful result through the existing response filtering path;
- preserve the resolved response URL and request URL-list state; and
- avoid invoking any provider for an unsupported scheme.

When the first handler returns an `InternalResponse`, extract common response
finalization before adding more handlers. Do not duplicate CORS, response
policy, service authorization, cookie processing, or exposure logic in scheme
handlers.

### 3. Implement `data:` fetching

Add a focused `data:` parser/handler that supports:

- percent-encoded payloads;
- `;base64` payloads;
- media-type parameters;
- the default media type when one is omitted;
- empty payloads; and
- typed errors for malformed percent encoding, invalid base64, or invalid
  media-type syntax.

Construct the result as an internal response with status 200, the resolved
`data:` URL, decoded bytes in the shared `Body`, and the generated
`Content-Type`. Assign the scheme's required opaque-origin behavior and do not
attach cookies or other credentials. `data:` responses should not enter the
HTTP cache.

Add tests for plain text, percent encoding, base64, media-type parameters,
default type, empty payloads, malformed encoding, invalid base64, body bytes,
response URL, origin, and credential behavior.

### 4. Implement `blob:` resolution

Call the browser-owned blob provider and convert its result into an internal
response. The handler must:

- fail when the URL is missing or revoked;
- use the provider-supplied creator origin;
- preserve the blob media type;
- use the shared body and null-body rules;
- avoid HTTP transport and cookies; and
- map provider failures to documented typed errors.

Add tests for successful resolution, missing/revoked blobs, media type,
creator origin, credential exclusion, response filtering, and body
consumption.

### 5. Implement `file:` access through a provider

Require the explicit filesystem/security provider. The handler must never read
the filesystem directly. The provider defines path resolution, authorization,
origin assignment, media type, and failure behavior.

Add tests for absent providers, denied access, missing files, provider
failures, successful reads, origin assignment, credential behavior, response
headers, and body/null-body semantics. Do not add filesystem state or a file
registry to `net`.

### 6. Implement `byui:` page routing

Add a browser-owned page provider for registered local browser pages. Route
`byui:` paths only through this provider, never through Reqwest or the
filesystem.

The provider contract must define page lookup, path normalization, response
status, media type, headers, origin, and access-control behavior.

Add tests proving that registered pages resolve, missing pages fail, query and
path handling are deterministic, HTML metadata is preserved, response URLs and
origins are assigned correctly, and no network or filesystem access occurs.

### 7. Implement the supported `about:` cases

Define an explicit allowlist. Implement the required empty/blank response
behavior, beginning with the supported blank URL, and reject all other
`about:` URLs before provider or transport access.

Document and test the chosen status, media type, origin, response URL, and the
distinction between an empty readable body and a null body. Defer additional
`about:` variants until a browser caller requires them.

### 8. Generalize response metadata where HTTP assumptions remain

Update [`InternalResponse`](../src/api/response/view.rs) and related response
helpers only where necessary to support provider-assigned metadata. In
particular, do not derive every response origin with
`Origin::from_url` if the scheme is opaque or the provider supplies the
origin.

Keep public response exposure centralized. All scheme handlers must populate:

- response URL;
- response origin;
- status and status text where applicable;
- internal headers;
- response type;
- body; and
- null-body state.

Do not broaden the public response API or add raw transport/provider types.

### 9. Update documentation and regression coverage

Update [`crates/net/src/lib.rs`](../src/lib.rs) and the engine contract to say
which schemes are supported and which require browser providers. Update the
unsupported-feature table in [`fetch-engine-test-plan.md`](fetch-engine-test-plan.md)
so it no longer treats all non-HTTP schemes as unsupported.

Add a dedicated `crates/net/tests/schemes.rs` suite covering:

- HTTP(S) regression behavior;
- each supported scheme;
- malformed and unsupported URLs;
- provider absence and provider denial;
- response URL and origin assignment;
- credential and cookie exclusion;
- response filtering;
- empty, readable, and null-body behavior; and
- no-network/no-unrelated-provider guarantees.

Finish with `cargo fmt --all`, the focused `net` tests, the workspace-relevant
`cargo test` commands, and `cargo clippy --all-targets -- -D warnings`.

## Feature 4.4 completion criteria

Feature 4.4 is complete when HTTP(S) behavior is unchanged, supported local
schemes never invoke the HTTP transport, unsupported schemes fail before side
effects, provider ownership is explicit, and every scheme reaches the same
internal/public response filtering and body pipeline.
