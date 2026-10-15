# Feature 4.4 plan: scheme-specific Fetch dispatch

Status: dispatch, provider contracts, provider metadata validation, shared
response finalization, the engine-owned `data:` handler, the `blob:` handler,
and focused scheme coverage are implemented; file, byui, and about handlers plus browser
composition remain. Reviewed against the current `crates/net` implementation
on 2026-10-15.

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
- Moved the HTTP(S) handler under `engine/schemes/http.rs` so all scheme
  handlers share one module boundary.
- Restricted cache eligibility and cache partition lookup to HTTP(S).
- Added unit coverage for scheme classification, provider-resource validation,
  and bytes/stream/null resource bodies.
- Added `tests/schemes.rs` coverage for supported `data:` forms and failures,
  `HEAD` null bodies, opaque no-CORS views, local-scheme cookie/cache/provider
  exclusion, unsupported-scheme side effects, provider capability boundaries,
  and rejected `about:` URLs.

The `data:` branch now decodes percent-encoded and base64 payloads into the
shared response/body pipeline. Provider-backed branches fail with a typed
`MissingProvider` error when their matching capability is absent; installed
providers still await their scheme handlers. Provider error categories and
scheme-resource validation now fail closed before public response exposure,
and HTTP and engine-owned scheme responses use shared response finalization.

The latest boundary pass also adds `ProviderErrorKind` categories for missing,
revoked, denied, I/O, malformed-metadata, and invalid-origin outcomes, plus
unit coverage for stable error formatting and scheme/media-type validation.

## Next steps

### 1. Define the scheme and provider boundary

The URL classification and dispatcher seam are complete. The provider API,
controller bundle, stable provider error categories, scheme-resource metadata
validation, and common response finalization are also complete. Browser
composition and provider-boundary integration coverage remain before any
provider-backed scheme handler can be considered complete.

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

#### 1.2 Define narrow provider traits — complete

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

#### 1.3 Decide how providers enter `FetchServices` — complete

Choose and document one integration shape:

- optional provider accessors on [`FetchServices`](../src/api/services.rs); or
- a separate browser-owned provider bundle installed alongside
  `FetchServices`.

A separate [`SchemeProviders`](../src/api/providers/mod.rs) bundle is selected
so scheme capabilities do not broaden `FetchServices`. The controller receives
it through `RequestController::with_services_and_providers`; there is no global
registry lookup.

#### 1.4 Preserve existing service implementations — complete

New provider accessors should have default implementations that return
`UnsupportedFeature`, or providers should be optional in the bundle. This
keeps existing test and standalone service implementations source-compatible
until they opt into a scheme.

Document which missing-provider errors occur before any provider or transport
side effect and which provider failures may occur after a resource has been
opened.

#### 1.5 Define failure and security semantics — implemented boundary

`ProviderErrorKind` and `RequestError::Provider` now define stable categories
for:

- provider absent;
- resource missing;
- blob revoked;
- file access denied;
- malformed provider metadata; and
- invalid or unavailable provider-assigned origins.

Providers still own authorization, sandboxing, path resolution, revocation, and
durable state. `net` fails closed when a required provider or security decision
is unavailable. End-to-end provider failures remain untested until the actual
handlers exist.

#### 1.6 Wire providers into controller state — complete

[`ControllerInner`](../src/engine/controller.rs) stores the optional,
shareable [`SchemeProviders`](../src/api/providers/mod.rs) bundle, and
`RequestController::with_services_and_providers` installs it without owning a
registry.

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

#### 1.8 Add provider-boundary tests — partial

Existing tests cover missing-provider errors, installed blob resolution and
typed blob failures, unsupported-scheme no-side-effect behavior, provider
error-category display, scheme-resource scheme/media-type/status validation,
and buffered, streamed, and null provider-resource bodies. Remaining tests must
prove that:

- each implemented provider handler calls only its matching provider exactly
  once;
- provider metadata reaches response exposure without leaking provider state;
- browser-context composition uses `with_services`; and
- existing `FetchServices` implementations remain source-compatible.

These tests belong beside the service/dispatcher integration tests. Tests for
actual decoding, file reads, blob resolution, page lookup, and `about:` body
semantics belong to the later scheme-handler steps.

The supported scheme classification remains:

```text
Http, Https, Data, Blob, File, Byui, About
```

#### 1.9 Make provider outcomes and metadata validation explicit — implemented boundary

The provider boundary now defines stable, testable outcomes rather than
returning only an undifferentiated provider failure. It distinguishes:

- provider absent, before provider lookup or other side effects;
- resource missing or a blob URL revoked;
- file access denied by authorization or sandbox policy;
- provider I/O or lifecycle failure after the provider has started;
- malformed provider metadata; and
- invalid, unavailable, or disallowed provider-assigned origins.

`ProviderErrorKind` categories are carried by `RequestError::Provider`; the
provider name is stable routing metadata and diagnostic text does not expose
provider implementation details.
Do not expose filesystem, registry, storage, or provider implementation
details through the error API.

Before constructing an `InternalResponse`, `net` now validates:

- the returned URL is syntactically valid and compatible with the selected
  scheme;
- status and status text are valid for the response representation;
- header names and values are valid and remain in the internal header channel;
- media type metadata is valid and agrees with the selected content type;
- the origin is either an explicitly permitted opaque origin or a valid
  provider-assigned origin; and
- provider metadata cannot replace the request's scheme, URL-list state, or
  credential policy.

Malformed or unavailable security metadata fails closed before public response
exposure. Full provider authorization and resource-existence behavior remains
dependent on the not-yet-implemented provider handlers.

#### 1.10 Define browser composition and provider lifecycle contracts

Document the concrete composition path outside `net` before enabling a
provider-backed handler:

- `crates/browser` constructs the controller with browser services and the
  provider bundle;
- the page provider owns page registration, lookup, path normalization, and
  page revocation or replacement rules;
- the blob provider owns object-URL creation, creator-origin association,
  revocation, and lifetime behavior; and
- the file provider owns path resolution, sandbox authorization, platform
  differences, and file-resource lifetime.

Composition tests must prove that an installed provider is selected by scheme,
called exactly once, and receives the prepared request context. They must also
prove that a request cannot call an unrelated provider or fall back to HTTP or
direct filesystem access when lookup fails.

Provider operations must define cancellation and lifetime behavior. Cancellation
before resolution must prevent the provider call when possible; cancellation
after resolution must terminate a provider stream and release engine resources.
Dropping, failing, or aborting a provider body must not create a cache entry or
leave a provider operation running.

### 2. Replace the remaining dispatcher placeholders with scheme handlers

The controller-level seam and shared response finalization now exist in
[`RequestController::fetch`](../src/engine/execute.rs). Replace its non-HTTP
placeholder branches with handlers in this order:

1. `blob:` provider resolution.
2. `file:` provider-authorized access.
3. `byui:` browser-page routing.
4. The explicitly supported `about:` cases.

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

`finalize_response` and `finalize_scheme_resource` in
[`process_response.rs`](../src/engine/process_response.rs) now centralize CORS,
response policy, service authorization, and public exposure preparation. New
scheme handlers must use these helpers and must not duplicate CORS, response
policy, service authorization, cookie processing, or exposure logic.

The common finalization operation must be explicit and reusable. It must:

1. validate and normalize provider or engine-owned response metadata;
2. assign the response URL-list, origin, status, headers, body, and null-body
   state;
3. select the response type using the request mode and assigned origin;
4. apply response policy and browser service authorization;
5. leave cookie processing to the scheme-specific policy boundary: HTTP may
   select/store cookies when credentials allow it, while local schemes bypass
   cookies entirely;
6. apply the existing public response filtering; and
7. apply cache behavior only for schemes whose contract explicitly permits it.

Local schemes must not obtain cookies, use HTTP cache entries, or invoke HTTP
transport unless a later plan section explicitly changes their contract.

### 3. Implement `data:` fetching — complete

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

The parser and handler are implemented and covered for plain text, percent
encoding, base64 (including an empty payload), media-type parameters, the
default media type, empty payloads, commas, non-UTF-8 bytes, malformed percent
encoding, invalid or duplicate base64 markers, malformed media types and
parameters, body bytes, response metadata, and service behavior. `HEAD` now
preserves response metadata while exposing an explicit null body. The suite
also proves that `data:` uses the shared response filtering path, can become
opaque in no-CORS mode, and bypasses cookies, HTTP cache partitioning, and all
scheme providers.

`data:` parsing intentionally materializes bytes, so streaming-output coverage
belongs to provider-resource tests and the future provider-backed handlers.

### 4. Implement `blob:` resolution — complete

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

The focused suite now distinguishes an absent provider from revoked/provider
metadata failures, verifies creator-origin metadata through response services,
covers buffered, streamed, and explicit-null bodies, and proves that blob
responses bypass cookies and HTTP cache partitioning. Cancellation and
body-drop coverage remains part of the broader provider lifecycle work below.

### 5. Implement `file:` access through a provider

Require the explicit filesystem/security provider. The handler must never read
the filesystem directly. The provider defines path resolution, authorization,
origin assignment, media type, and failure behavior.

Add tests for absent providers, denied access, missing files, provider
failures, successful reads, origin assignment, credential behavior, response
headers, and body/null-body semantics. Do not add filesystem state or a file
registry to `net`.

Provider-denied, missing, malformed-path, and provider-I/O outcomes must use
the documented error categories. Tests must cover platform/path normalization
through the fake provider contract rather than host filesystem behavior, and
must prove that invalid provider origins or metadata fail before exposure.

### 6. Implement `byui:` page routing

Add a browser-owned page provider for registered local browser pages. Route
`byui:` paths only through this provider, never through Reqwest or the
filesystem.

The provider contract must define page lookup, path normalization, response
status, media type, headers, origin, and access-control behavior.

Add tests proving that registered pages resolve, missing pages fail, query and
path handling are deterministic, HTML metadata is preserved, response URLs and
origins are assigned correctly, and no network or filesystem access occurs.

Also test page registration and replacement/revocation behavior, malformed
page metadata, unsupported page kinds, exact-once provider calls, and
cancellation or body failure after page resolution.

### 7. Implement the supported `about:` cases

Define an explicit allowlist. Implement the required empty/blank response
behavior, beginning with the supported blank URL, and reject all other
`about:` URLs before provider or transport access.

Document and test the chosen status, media type, origin, response URL, and the
distinction between an empty readable body and a null body. Defer additional
`about:` variants until a browser caller requires them.

The allowlist decision must be made before provider lookup, cookie lookup,
cache access, or transport admission. Test every rejected spelling, including
unexpected query and fragment combinations, and test the accepted method
boundary, response type, and body lifecycle.

### 8. Generalize response metadata where HTTP assumptions remain — boundary implemented

[`InternalResponse::from_scheme_resource`](../src/api/response/view.rs) now
accepts the selected scheme, validates scheme-compatible URLs, status text,
media-type metadata, and provider origins, and preserves opaque or
provider-assigned origins. Do not derive every response origin with
`Origin::from_url` if the scheme is opaque or the provider supplies the origin.

Keep public response exposure centralized. All scheme handlers must populate:

- response URL;
- response origin;
- status and status text where applicable;
- internal headers;
- response type;
- body; and
- null-body state.

Do not broaden the public response API or add raw transport/provider types.
Remaining work is end-to-end validation through real blob/file/byui handlers
and tests for provider-origin and metadata failures.

### 9. Update documentation and regression coverage

Update [`crates/net/src/lib.rs`](../src/lib.rs) and the engine contract to say
which schemes are supported and which require browser providers. Update the
unsupported-feature table in [`fetch-engine-test-plan.md`](fetch-engine-test-plan.md)
so it no longer treats all non-HTTP schemes as unsupported.

The contract documentation must also record provider error categories,
metadata validation rules, the browser composition path,
cancellation/lifetime behavior, cookie/cache exclusions, and whether each
scheme returns an empty readable body or an explicit null body for `HEAD` and
other bodyless methods.

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

The suite must include recording providers and service fakes that assert:

- installed providers are invoked exactly once;
- absent providers fail before provider or transport side effects;
- unrelated providers are never invoked;
- provider metadata is validated before exposure;
- provider errors retain their documented category;
- cancellation and body drop release provider and scheduler resources; and
- successful local responses use the same finalization and exposure helper as
  HTTP(S), without entering the HTTP cache.

Finish with `cargo fmt --all`, the focused `net` tests, the workspace-relevant
`cargo test` commands, and `cargo clippy --all-targets -- -D warnings`.

## Feature 4.4 completion criteria

Feature 4.4 is complete when HTTP(S) behavior is unchanged, supported local
schemes never invoke the HTTP transport, unsupported schemes fail before side
effects, provider ownership and browser composition are explicit, every
installed provider is selected exactly once by its matching scheme, provider
outcomes and metadata are validated fail-closed, cancellation and body
lifecycle rules are covered, and every successful scheme reaches the same
internal/public response filtering and body pipeline. Completion also requires
the dedicated scheme matrix, browser composition tests, focused `net` tests,
workspace-relevant tests, formatting, and clippy with warnings denied.
