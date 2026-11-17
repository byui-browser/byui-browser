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

## Policy matrix

This matrix is the intended contract for the refactor. “Initial” means the
single preparation pass before scheme dispatch. “Hop” means the initial HTTP
exchange and every followed HTTP redirect target. A denied decision or service
error always stops the fetch with its typed `RequestError`; no permissive
fallback is permitted.

| Rule | Owner | Applies to | Cadence | Required ordering and result | Missing or denied behavior |
| --- | --- | --- | --- | --- | --- |
| Method, mode, body, and caller-header invariants | `Request` construction | All caller-created requests | At mutation time | Reject forbidden methods, invalid no-CORS combinations, disallowed bodies, and guarded headers before execution state exists | Return the setter error; no controller work occurs |
| URL parsing, base-URL resolution, feature support, and scheme selection | Core Fetch policy | All requests | Initial | Complete before providers, services, cache, cookies, or transport | Return the policy error; no side effects |
| Initial request authorization | Browser request-policy service | All prepared requests | Initial once | Run after core validation and before service-worker, cache-partition, provider, cache, cookie, or transport work | A request requiring browser services without an installed service fails closed; a service denial or error propagates |
| Service-worker interception decision | Browser service-worker service | Eligible initial requests; excludes `file:` | Initial once | Run after initial authorization and before cache partition, cache lookup, cookies, providers, or transport | `Intercept` fails closed until worker response execution exists; absent browser services follow the initial authorization rule |
| Cache partition selection | Browser cache-partition service | HTTP and HTTPS requests | Initial once | Run after initial authorization and service-worker selection; attach the partition before either preflight or HTTP cache use | Service error propagates; no implicit durable or cross-context partition is selected |
| Local-provider selection and provider authorization | Scheme-provider registry and provider | `blob:`, `file:`, and `byui:` | Initial once | Run only after core validation and initial authorization; provider selection occurs exactly once | Missing, revoked, malformed, or denied capabilities fail closed; never fall back to HTTP or direct filesystem access |
| Redirect transition | Core Fetch policy | Followed HTTP redirects | Once for each redirect response | Finish and approve the redirect response, then apply method/body rewrite, sensitive-header stripping, referrer-policy update, URL-list update, and service-worker disablement before validating the next hop | Invalid location, redirect-mode refusal, limit exhaustion, or non-replayable body failure returns a redirect error |
| Redirect-target validation | Core Fetch policy | Followed HTTP redirects | Once for each next hop | Validate the parsed target after transition and before authorization, generated headers, cache lookup, cookies, preflight, or transport | Return the target-policy error; do not submit transport work |
| Per-hop request authorization | Browser request-policy service | HTTP and HTTPS hops | Once per hop | Run after target validation and before cache lookup, cookies, preflight, or transport | Service denial or error propagates; do not reuse an earlier-hop decision |
| Referrer reduction and Fetch-owned headers | Core Fetch policy | HTTP and HTTPS hops | Once per hop | Reduce referrer against the current target, then derive Fetch-owned headers before browser-generated headers are merged | Header-construction errors propagate; browser headers cannot replace protected Fetch-owned fields |
| Browser-generated headers | Browser header service | HTTP and HTTPS hops | Once per hop | Merge after Fetch-owned headers and before CORS-preflight calculation so the preflight represents the actual transport request | Service or header-guard error propagates; absent service yields no browser-generated headers |
| CORS preflight | Core Fetch policy and HTTP transport | Cross-origin CORS hops requiring preflight | Once per applicable hop, subject to the partitioned permission cache | Run after all transport headers are known and before cookie attachment and the main transport request | A cache miss, transport failure, malformed permission, or denial fails the main request; preflight responses are not exposed or passed to ordinary response policy |
| Cookie selection | Browser cookie service | Credential-permitted HTTP and HTTPS hops | Once per hop after cache miss | Run after validation and preflight approval, then attach only an internally controlled `Cookie` header | Service error or invalid returned header fails the request; local schemes never invoke this service |
| HTTP cache lookup | Core cache adapter with browser partition | Cacheable HTTP and HTTPS hops | Once per hop before cookie selection and transport | Use the partition selected during initial preparation; a hit produces an internal response for normal finalization | `OnlyIfCached` without a permitted hit fails as a network error; local schemes bypass the HTTP cache |
| Transport or provider execution | HTTP transport or selected provider | Validated HTTP hops or selected local schemes | Once per attempt | Receive only validated state; providers do not perform HTTP policy, cookie, cache, or unrelated-provider work | Transport and provider errors propagate as typed fetch failures |
| Response classification and generic validation | Core Fetch policy | Network, cache, provider, `data:`, `about:`, and manual-redirect responses | Once for every response that may be exposed | Classify CORS response type, or set opaque-redirect type for a manual redirect, then run generic response validation before browser approval | Validation failure rejects the response and clears matching preflight permission after a failed preflighted HTTP fetch |
| Browser response authorization and diagnostics | Browser response-policy service and observer | Every response that may be exposed | Once per response | Run after core classification and before public filtering/exposure; notify diagnostics only after approval | Service denial or error rejects the response; observers remain non-failing and do not alter the outcome |
| Public filtering and exposure | Core Fetch policy | Every approved exposed response | Once per response | Remove internal-only headers and expose the response type selected by finalization | No public response is created until every preceding response-policy step succeeds |
| `Set-Cookie` processing | Browser cookie service | Approved credential-permitted HTTP and HTTPS responses, including approved manual redirects | Once per internal response | Run after response approval and before cache storage; mark processed responses to prevent duplicate storage | Service error rejects the fetch; local and cache-served responses do not write cookies again |
| Cache insertion and body capture | Core cache adapter | Approved cacheable HTTP and HTTPS responses | Once per final response body completion, or immediately for null bodies | Run only after approval and cookie processing; cache hits never reinsert themselves | Capture failure, dropped body, and failed body stream do not create a cache entry |

### Redirect and service-worker rule

Service-worker interception is an initial-request decision. A followed network
redirect retains per-hop request authorization but disables service-worker
interception before the next hop. This is intentional: the current engine can
detect an initial interception but cannot execute a worker-produced response,
and it must not attempt to re-intercept a request that has already entered the
network redirect algorithm.

### Scheme and response coverage

| Response source | Initial authorization | Per-hop authorization | HTTP cache and cookies | Response finalization |
| --- | --- | --- | --- | --- |
| HTTP network response | Yes | Yes, once per hop | Yes when applicable | Yes |
| HTTP cache hit | Yes | Yes for the hop that looked up the entry | Cache lookup only; no cookie read or `Set-Cookie` write | Yes |
| HTTP manual redirect | Yes | Yes for the hop that received it | `Set-Cookie` may run after approval; no cache insertion | Yes, as opaque redirect |
| CORS preflight response | Initial and per-hop request rules only | Yes for its parent hop | Preflight permission cache only; no cookies or HTTP response cache | No public-response finalization |
| Provider-backed local response | Yes | Not applicable | Never | Yes |
| `data:` or `about:` response | Yes | Not applicable | Never | Yes |

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

The policy matrix above is the implementation contract. Keep it synchronized
with the Fetch service API, redirect behavior, scheme-provider contracts, and
test coverage. Record a separate ADR if a future public capability split
changes ownership or fail-closed behavior.

### 2. Introduce explicit internal states

Implemented: `PreparedRequest` represents a request whose initial URL
validation, scheme selection, initial browser authorization, service-worker
evaluation, and HTTP cache-partition setup have completed.

Implemented: `ValidatedHop` is now the only state accepted by the scheduler
and HTTP transport. It represents an HTTP hop that has completed:

- redirect-target validation;
- per-hop browser authorization;
- referrer reduction;
- Fetch-owned and browser-generated header assembly;
- CORS preflight, when required; and
- cookie selection, when credentials permit it.

Internal CORS preflight exchanges use the same transport boundary with an
engine-generated preflight request. Step 3 will centralize the code that
constructs ordinary HTTP `ValidatedHop` values without changing this boundary.

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
