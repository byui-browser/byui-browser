# Browser `file:` provider

The browser adapts security-owned file access to `net::FileProvider`.
Security owns filesystem capabilities and path authorization; the browser owns
request context, grants, limits, and session lifecycle; `net` owns dispatch,
response validation, and body behavior.

## Current status

The provider is default-deny and capability-backed for trusted directory
selections. It uses retained directory handles with symlink following disabled,
browser-issued single-use navigation authorization, bounded 64 KiB streaming,
and bounded reader admission. Per-file size limits, allowlisted media types,
captured `Content-Length` enforcement, request cancellation, and revocable
generation-bound grants are implemented. Revocation prevents future opens and
body delivery, including already queued chunks.

Each admitted `file:` document receives an opaque origin. Local-file access is
limited to browser-issued top-level navigations; all subresources, workers, and
nested browsing contexts remain denied until document/container identity is
available end-to-end.

Potential future work is a scoped `file:` origin in `net`, bound to the
security-owned capability and browser document/container rather than a global
`file:` origin. That identity could safely support same-scope local
subresources, module loading, workers, nested contexts, and same-origin fetch
checks. A bare shared `file:` origin must not be added because it would make
unrelated local files same-origin.

The public trusted-selection constructor remains a platform trust boundary; no
native directory picker is provided by this crate. Renderer, web-content, and
ordinary request-construction code must not receive selections or
`FileAccessCapability` values.

## Remaining work

### 1. Complete resource and active-content limits

Define and enforce maximum read duration, total bytes served per session,
aggregate blocking-reader/channel memory, and platform-specific I/O limits.

Define MIME sniffing, `nosniff`, and executable-content rules before local
HTML, JavaScript, images, stylesheets, or other active content is interpreted.
Decide whether directories and unsupported types retain the current
non-diagnostic error categories or receive dedicated errors. Test each limit
at its boundary, including slow reads, contention, queue exhaustion, and
cancellation.

### 2. Define mutable-file and stream-consistency semantics

Choose whether responses require a stable snapshot or may observe live file
contents. Document and test the behavior for in-place writes, truncation,
growth, and metadata changes during setup and streaming, including response
framing and cleanup of retained handles, channels, and permits.

### 3. Complete permission lifecycle behavior

Add grant expiry, audit logging, and explicit process-shutdown sequencing. Use
the existing grant-revocation path for expiry so pending setup, live readers,
and queued chunks behave consistently with close, navigation-away, and grant
replacement.

Document denied operations after expiry, whether existing bodies are aborted,
which lifecycle events are recorded, and exclude sensitive paths, contents,
and OS diagnostics from logs. Test expiry, replacement,
controller destruction, shutdown, pending setup, active readers, queued data,
and navigation authorizations.

### 4. Maintain regression and integration guarantees

Keep deterministic regression coverage for blocking setup, response
publication, queued chunks, worker-side failures, full-channel backpressure,
four-reader concurrency, queue exhaustion, request aborts, and `GET`/`HEAD`
revocation behavior. Verify every path releases handles, permits, channels,
and blocking tasks.

Keep cross-crate tests aligned with these guarantees:

- Local-file requests never use HTTP transport, cookies, service workers, or
  the HTTP cache.
- Unrelated scheme providers cannot satisfy a `file:` request.
- Provider errors stay stable and non-diagnostic.
- Authorization occurs before path conversion for unauthorized requests.
