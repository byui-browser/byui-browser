# Feature 4.4 plan: scheme-specific Fetch dispatch

Status: the net-level implementation is complete for the current scheme
slice. `data:`, `blob:`, `file:`, `byui:`, and `about:blank` dispatch through a
shared response pipeline. Provider contracts, metadata validation, typed
provider errors, local-scheme HTTP/cache/cookie exclusion, `GET`/`HEAD`
enforcement, response filtering, null-body handling, cancellation, and
body-drop cleanup are implemented and tested. The browser installs its
embedded `byui:` provider through `RequestController::with_providers`.

This document tracks only the remaining work for Feature 4.4. Full CORS,
redirect, cookie, cache, and body-cloning behavior remains in the broader
Fetch plans.

## Remaining work

### 1. Finish browser-owned composition

Complete browser-side ownership and composition without moving browser state
into `net`:

- provide a concrete browser `FetchServices` implementation and compose it
  with `SchemeProviders` through `with_services_and_providers` when browser
  client context is available;
- keep `byui:` page registration, lookup, replacement, and origin policy in
  `crates/browser`;
- add browser/storage ownership for blob URL creation, creator origins,
  revocation, and lifetime; and
- add the designated filesystem/security owner for authorized `file:` access,
  sandboxing, path resolution, and origin policy.

Browser-owned providers must never fall back to HTTP or direct filesystem
access when lookup fails.

### 2. Complete the `byui-provider` implementation

The current provider is a small declarative embedded-page registry. Finish the
provider-local hardening work:

- avoid copying embedded resource bytes into a fresh `Vec` for every request;
- retain the engine-enforced `GET`/`HEAD` boundary and explicit `HEAD` body
  semantics;
- keep route lookup exact and declarative, including declared resource
  aliases and unknown-host/path rejection; and
- document and enforce the chosen opaque or stable browser-UI origin policy.

Add focused browser tests for method rejection, `GET`/`HEAD` bodies, query and
fragment handling, aliases, repeated loads, shared-body ownership, unknown
routes, and origin policy.

### 3. Expand provider conformance coverage

Add browser-composition and net integration tests for:

- `with_services_and_providers` and request-context propagation;
- exact-once selection of the matching provider;
- missing, revoked, denied, malformed, and I/O-failure outcomes;
- provider-assigned tuple and opaque origins;
- response headers, status, URL, media type, empty/readable/null bodies;
- cancellation while resolving and streaming, including resource cleanup;
- no HTTP transport, cookie, cache, or unrelated-provider calls; and
- repeated resolution of the same resource without accidental HTTP caching.

### 4. Maintain the supported-scheme boundary

Keep `data:` parsing engine-owned and retain the exact `about:blank`
allowlist. Unsupported schemes must fail before service, provider, cache, or
transport side effects. Any new local scheme must define its provider owner,
origin policy, method policy, body/null-body behavior, response filtering, and
cache/cookie behavior before implementation.

### 5. Verification and documentation

Keep the feature contract, test plan, and standard-gap audit synchronized with
the implementation. Before considering the browser-level feature complete,
run the focused net and browser tests, workspace-relevant tests,
`cargo fmt --all`, and `cargo clippy --all-targets -- -D warnings`.

## Completion criteria

Feature 4.4 is complete when browser-owned providers are composed explicitly,
the `byui:` provider is hardened and tested, concrete blob/file ownership is
available outside `net`, every provider-backed scheme has the conformance
matrix above, and all local-scheme isolation and lifecycle guarantees remain
verified.
