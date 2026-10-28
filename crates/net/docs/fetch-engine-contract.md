# Fetch engine contract (first slice)

`net` is the reusable Rust Fetch engine. It owns request preparation, response
tainting and exposure, body transport, cancellation, process-local scheduling,
and coordination of cache, credential, security, and transport services. It does
not create JavaScript objects or promises. `webapis` owns that conversion; its
current binding returns a string and is not yet a complete Fetch binding.

`RequestController::fetch` returns a filtered `Response` as soon as response
headers pass Fetch filtering. Its abort-aware `Body` holds the scheduler permit
until it is consumed or dropped. Body errors may occur after headers. Callers
cannot obtain the transport response.

`Body` uses one lifecycle state for helper locking, direct streaming, transport
handoff, completion, failure, and abort. Nullness is part of that model rather
than a parallel response flag. Declared length and media type survive live,
cached, and provider-backed responses. `Response::try_clone` tees an unused
live body through bounded one-chunk branches; a slow active branch applies
backpressure, dropping a branch cancels it, and the shared scheduler permit is
released only after the source completes or both branches are dropped. Cache
capture is bounded by `Config::max_cache_body_size` and commits only at a clean
end-of-stream.

Error timing and retry behavior are part of this boundary:

| Error category                                                                  | Timing                  | Body effect and recovery                                                                                          |
| ------------------------------------------------------------------------------- | ----------------------- | ----------------------------------------------------------------------------------------------------------------- |
| URL, method, header, scheme, capability, keepalive, and request-body validation | Before headers          | No response body exists; a corrected request may be sent.                                                         |
| Same-origin, CORS, and redirect failure                                         | Before public headers   | The internal response is discarded; retry only with a new, permitted request.                                     |
| Client initialization                                                           | Before any request      | Correct the configuration; no body was used.                                                                      |
| Cache-only network miss                                                         | Before headers          | No body exists; retry after cache state changes or change cache mode.                                             |
| Abort                                                                           | Before or after headers | The active stream terminates; a new request needs a new signal.                                                   |
| Transport failure                                                               | Before or after headers | A one-shot request body may be spent; a delivered response stream is spent. Retry with a new, replayable request. |

The private `InternalResponse` retains the transport status and canonical
reason phrase, every header (including `Set-Cookie` and obsolete `Set-Cookie2`), complete URL-list and
redirect state for the supported exchange/redirect slice, origin and tainting state,
cache provenance, cookie-processing state, and the live body, whose own state
authoritatively represents nullness.
Policy and cookie processing run on that representation. Exposure constructs
an immutable header list and removes forbidden response headers, including the
obsolete `Set-Cookie2` compatibility case. CORS views
include only safelisted and explicitly exposed headers. Opaque views hide
status, URL, headers, cache provenance, and body. `fetch` uses this conversion. An opaque
view drops the internal body stream and releases its scheduler permit; this
slice does not continue downloading an unreadable body in the background.

The public response status is a `u16`, and its headers are `Headers`; no
`reqwest::StatusCode`, `reqwest::HeaderMap`, `reqwest::Response`, or transport
stream type crosses the response boundary. Engine request APIs use `http` and
`url` types rather than Reqwest re-exports. Controller construction and fetch
failures use `RequestError`; transport errors expose a diagnostic string rather
than a Reqwest error type. `Response` exposes the shared, potentially live,
abort-aware `Body` type.

`Headers` is the shared request and response header model. String constructors
and mutation methods validate HTTP names and values, lowercase names, and trim
leading and trailing HTTP whitespace from values. The raw representation keeps
duplicates in insertion order for transport and policy code. Public `get` and
iteration combine duplicate ordinary fields with a comma and space in the order
received; `Set-Cookie` values remain separate and are available to trusted
internal consumers through `get_set_cookie`. Request, no-CORS request, response,
and immutable guards may only transition toward stricter states. Immutable
mutation and guard-loosening attempts return stable typed errors.

Caller request headers remain separate from the transport header channel.
Generated body metadata, `Origin`, `Referer`, and cookies are added only while
preparing the transport request. Internal response headers retain every field
for `FetchServices`, while public basic responses remove cookie fields and CORS
responses expose only safelisted or valid explicitly exposed fields. The CORS
wildcard exposes non-cookie fields only when credentials mode is not `Include`;
malformed exposure metadata exposes no additional fields. Opaque,
opaque-redirect, and error views always construct empty immutable headers,
although creating opaque redirects and error responses remains later algorithm
work.

## Supported behavior and capability boundaries

- HTTP(S), relative URL resolution, request method/header guards, replayable
  and one-shot input bodies, abort signals, simple same-origin and CORS checks,
  basic/opaque response views, and a small process-local cache are supported.
- `data:` and the exact `about:blank` URL are engine-owned local schemes.
  `blob:`, `file:`, and `byui:` are supported when their matching
  `SchemeProviders` capability is installed. Other `about:` URLs and
  unclassified schemes fail before provider or transport side effects.
- Local schemes never use HTTP transport, cookie selection or storage, or the
  HTTP response cache. Successful local results still pass through the common
  response policy, browser-service approval, filtering, and body pipeline.
- Provider-backed `blob:`, `file:`, and `byui:` resources are read-only:
  `GET` and `HEAD` are accepted before provider lookup; other methods fail with
  `UnsupportedFeature` and cannot invoke a provider.
- Provider resources carry only response URL, status, headers, media type,
  origin, and body data. The engine rejects wrong-scheme URLs, invalid status
  metadata, conflicting media types, and invalid origins before public
  exposure. Missing capabilities use `MissingProvider`; provider failures use
  stable `ProviderErrorKind` categories without exposing implementation data.
- `GET` may expose provider bytes or a provider stream. `HEAD`, status 101,
  204, 205, and 304 always expose a null body even if a provider supplied body
  data. An explicit `SchemeBody::Null` remains null for every method.
- Provider resolution futures are dropped when a request is aborted. Returned
  byte and streaming bodies observe the same abort signal; aborting, failing,
  or discarding a body drops its provider stream. Providers must release their
  own handles, tasks, and other resources when those futures or streams drop.
- `RequestController::builder` is the construction API. Its optional
  `with_fetch_services` and `with_scheme_providers` setters compose browser
  services and local-scheme capabilities. A controller without services is for
  standalone requests without client context. A request carrying browser
  client context requires browser-owned `FetchServices`; local schemes require
  their matching provider. That interface supplies security decisions, credential
  selection and storage, a service-worker network decision, a cache partition,
  and a response-header observation hook. `Set-Cookie` reaches only that
  trusted interface. Credential mode still decides whether the engine calls
  its credential methods.
- CORS preflight, integrity, navigation fetch, keepalive lifetime, nondefault
  priority scheduling, and cache revalidation fail with `UnsupportedFeature`.
  Manual/error redirect modes are handled by the controller. A service-worker
  `Intercept` decision also
  fails closed until worker responses are integrated. The default standalone
  service-worker mode means no worker is installed.
- Reqwest never follows redirects. The controller handles the supported
  redirect loop and returns filtered `OpaqueRedirect` results for manual mode;
  error mode fails before public headers. Per-hop policy checks remain
  incomplete. Fetch errors currently return a `RequestError`, not a
  `ResponseType::Error` value.
- `Default`, `NoStore`, `Reload`, `ForceCache`, and `OnlyIfCached` retain their
  documented process-local cache subset; `NoCache` fails until revalidation
  exists. Browser service providers choose the partition key. This cache is
  neither durable nor a full Fetch HTTP cache.
- Cache-only misses surface as Fetch network errors. Body stream transport
  errors and aborts retain separate typed errors.

`storage` owns durable cookie and cache data and network partition state.
`security` owns cookie, CSP, mixed-content, CORP, MIME, authentication, and
certificate decisions. Browser and service-worker crates own interception,
navigation/client context, and lifecycle. Diagnostics and timing consumers
own reporting. They supply decisions through `FetchServices`; none of their
databases belong here. Complete interception, persistence, and diagnostics
lifecycles remain work for their later plan steps.

Only owned strings, byte buffers, scalar metadata, and serializable request
context are suitable for Network/Renderer IPC. A controller, request body
stream, response body stream, abort signal, scheduler permit, and private
`InternalResponse` are process-local and must remain in the Network process.

Existing callers of raw response headers should use `Headers::get` or
ordered iteration. `Config.max_redirects` was removed because redirects cannot
be safely followed in this slice. Callers consume buffered and streaming
response bodies through `Body`; live bodies also implement `Stream` for Rust
consumers.
