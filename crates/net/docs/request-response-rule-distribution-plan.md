# Request and Response Rule Distribution Plan

The Fetch engine currently distributes request and response rules across public request setters, initial preparation, scheme handlers, the HTTP redirect loop, and response finalization. The implementation is broadly correct for the supported slice, but the lifecycle is implicit and makes it difficult to see which rules run once, which repeat after redirects, and which apply to local schemes.

This plan introduces explicit preparation and per-hop states, centralizes shared validation, and preserves the existing distinction between HTTP policy and local-scheme provider behavior.

## Current rule distribution

| Phase | Current location | Responsibility |
| --- | --- | --- |
| Request construction | `crates/net/src/api/request/mutators.rs` | Method, mode, body, and header-guard invariants |
| Initial validation | `crates/net/src/policy/request/validation.rs` | URL parsing, feature support, origin requirements, and scheme selection |
| Initial preparation | `crates/net/src/engine/prepare.rs` | Browser request authorization, service-worker decision, and cache partition selection |
| HTTP hop execution | `crates/net/src/engine/schemes/http.rs` | Redirect authorization, referrer reduction, headers, preflight, cookies, cache, and transport |
| Redirect mutation | `crates/net/src/api/request/redirect.rs` | Method/body rewriting, sensitive-header stripping, referrer policy, and redirect state |
| Response finalization | `crates/net/src/engine/process_response.rs` | CORS classification, response validation, browser response authorization, and exposure |
| HTTP post-processing | `crates/net/src/engine/schemes/http.rs` | `Set-Cookie` processing and cache insertion |

Local schemes share response finalization but intentionally do not participate in HTTP cookies, CORS preflight, or the HTTP cache.

## Target lifecycle

Document and enforce the following lifecycle:

1. Request construction establishes local invariants.
2. Initial preparation validates the request, selects a scheme, authorizes the initial request, evaluates service-worker interception, and attaches browser context.
3. Each executable HTTP redirect hop performs the same explicit per-hop validation.
4. Scheme execution receives only validated state and performs transport or provider work.
5. Every response passes through one response-finalization boundary before exposure.
6. HTTP-only effects such as `Set-Cookie` storage and cache insertion run only after response approval.

The lifecycle must explicitly record which checks run once, which repeat per redirect hop, and why service-worker interception is disabled after a network redirect.

## Implementation phases

### 1. Define the policy matrix

Add an architecture note or ADR containing the lifecycle above and a rule matrix for:

- request construction;
- initial preparation;
- per-hop validation;
- scheme execution;
- response finalization; and
- post-approval effects.

The matrix should identify whether each rule applies to HTTP, provider-backed local schemes, cached responses, manual redirects, or preflight requests. It should also define fail-closed behavior when browser services are absent or unsupported.

### 2. Introduce explicit internal states

Add an internal `PreparedRequest` state that guarantees initial URL validation, scheme selection, initial browser authorization, service-worker evaluation, and cache-partition setup are complete.

Add an internal `ValidatedHop` state that guarantees an HTTP hop has completed:

- redirect-target validation;
- per-hop browser authorization;
- referrer reduction;
- Fetch-owned and browser-generated header assembly;
- CORS preflight, when required; and
- cookie selection, when credentials permit it.

Transport should consume only validated hop state rather than a raw request.

### 3. Centralize per-hop validation

Create one controller-level per-hop validation path. The HTTP loop should call it for the initial HTTP hop and every followed redirect. It should own the ordering of:

1. abort checks;
2. redirect-hop request authorization;
3. referrer reduction;
4. Fetch-owned header construction;
5. browser-generated header merging;
6. CORS preflight; and
7. cookie selection.

The HTTP loop should then focus on cache lookup, transport submission, redirect interpretation, and response handling.

### 4. Separate redirect transitions from hop validation

Keep redirect mutation in `api/request/redirect.rs`, but make the transition explicit: method/body rewriting, sensitive-header removal, referrer-policy updates, redirect-count updates, and service-worker mode changes should be completed before the resulting request is passed through per-hop validation.

Preserve and document the current service-worker rule:

- the initial request is checked for interception;
- request authorization repeats for each followed network redirect;
- service-worker interception is disabled after the first network hop; and
- later redirects do not trigger a second interception attempt.

### 5. Unify response finalization

Make every response-producing path call one finalization pipeline responsible for:

1. response metadata construction;
2. CORS response classification;
3. generic response-policy validation;
4. browser-owned response authorization;
5. response diagnostics; and
6. public response filtering and exposure.

Run `Set-Cookie` processing and cache insertion only after this pipeline succeeds. Network responses, cached responses, manual redirects, and local-provider responses should all use the same finalization boundary.

### 6. Split browser capabilities after lifecycle stabilization

Keep the existing `FetchServices` trait stable during the internal refactor. Once call phases are explicit, split it along ownership boundaries such as:

- request policy;
- response policy;
- cookies;
- service workers;
- cache partitioning; and
- network observation.

This avoids designing the public interface around the current accidental call locations. The browser no-op implementation can then be replaced capability by capability.

## Regression coverage

Add focused tests for both behavior and call placement:

- initial request authorization runs exactly once;
- redirect request authorization runs once per followed HTTP hop;
- service-worker checks run only during initial preparation;
- generated headers are rebuilt for every validated HTTP hop;
- preflight sees the generated headers sent to transport;
- cookies are read only after request validation and before transport;
- response authorization runs for network, cached, manual-redirect, and local-provider responses;
- rejected responses do not process `Set-Cookie` or enter the cache;
- local schemes do not invoke HTTP cookie, cache, or preflight services; and
- cache hits cannot bypass response filtering or provider authorization.

The existing service-recording tests in `crates/net/tests/requests.rs`, preflight coverage, scheme tests, and redirect tests should be extended rather than replaced.

## Recommended sequencing

1. Document the lifecycle and policy matrix.
2. Add state types without changing behavior.
3. Centralize HTTP per-hop validation.
4. Centralize response finalization and post-approval effects.
5. Add call-count and ordering regression tests.
6. Replace browser no-op services one capability at a time.
7. Split `FetchServices` after the lifecycle is stable.
8. Run formatting, focused `net` and browser tests, workspace tests, and clippy with warnings denied.

This work should precede or accompany the browser credential and policy integrations described in `fetch-engine-implementation-plan.md`, because those integrations will otherwise add more policy decisions to already-distributed call paths.
