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

## Active-content policy

The browser uses the following conservative policy for local-file responses.
These decisions are part of the browser contract even where enforcement is
still listed as remaining work below.

### Default permission level

The default local-file mode is static top-level document rendering. A
browser-issued navigation may return an allowlisted static HTML document, CSS,
or passive raster image. The provider currently denies every local-file
subresource because it has no document/container identity; consequently,
linked stylesheets, images, fonts, and media are not yet loaded from local
files even when their types would otherwise be passive.

The default mode does not enable JavaScript, workers, service workers, nested
browsing contexts, popups, external network requests, cookies, HTTP-cache
state, local storage, IndexedDB, uploads, or arbitrary filesystem access.
Forms and other user-mediated operations remain denied unless a separate
browser policy explicitly authorizes them. Enabling script-capable local web
applications is a distinct, opt-in mode and must not be inferred from a file
extension.

- Media types are classified from a browser-owned extension allowlist. General
  MIME sniffing is not used initially; caller-controlled bytes must not change
  the declared media type. Any future sniffing must be a separate,
  security-reviewed policy component rather than ad hoc stream logic.
- Local responses with a declared media type include
  `X-Content-Type-Options: nosniff`. This reduces consumer-side
  reinterpretation but is not, by itself, a substitute for provider
  classification and active-content checks.
- Unknown extensions are treated as inert binary data. If the consuming path
  cannot guarantee inert handling, the provider denies the resource instead.
  Unknown content must never receive an inferred active media type.
- Native executable formats are denied regardless of filename extension where
  the platform can identify them. This policy does not claim to detect every
  possible executable or polyglot file.
- HTML and direct static CSS/image responses are enabled by the default mode.
  JavaScript, active SVG behavior, JSON, unknown files, and other
  script-capable or active formats require an explicit browser-owned
  active-content policy. The default remains deny for scripting until that
  policy is available end to end, including scripting and subresource rules.
- Directories and unsupported or disallowed files retain stable,
  non-diagnostic provider error categories. Trusted browser diagnostics may
  record a sanitized policy reason, but web-visible errors must not disclose
  sensitive paths, contents, or detailed OS information.

The policy is intentionally stricter than merely assigning an opaque origin.
An opaque origin prevents local documents from sharing a tuple origin, but it
does not by itself make active bytes safe to parse or execute. Enforcement must
occur before response metadata is published and must remain consistent with
the existing revocation and cancellation checks.

## Remaining work

### 1. Complete resource and active-content limits

The provider now enforces a 30-second default setup/stream deadline, a
256 MiB default cumulative body-byte budget per session, four active readers,
eight queued readers, and at most 512 KiB of queued payload (four readers,
two 64 KiB chunks each). Browser-owned configuration may tighten or raise the
file, session-byte, and duration limits; zero-valued limits are rejected.

Only static HTML/CSS and passive PNG, JPEG, GIF, and WebP responses are
admitted. Unknown, JavaScript, JSON, SVG, and other unsupported types are
denied, declared responses carry `X-Content-Type-Options: nosniff`, and a
bounded prefix check rejects recognizable native executable headers on the
current platform. This remains a provider policy, not a renderer capability:
the current provider still denies all local-file subresources and does not
authorize script interpretation.

Remaining work is to expose a configurable byte-buffer cap if channel sizing
becomes configurable, establish cancellable platform I/O deadlines rather
than enforcing the deadline at provider observation points, and add targeted
slow-I/O and full-channel timeout tests on each supported platform.

### 2. Define mutable-file and stream-consistency semantics

Responses use a retained open handle, not a filesystem snapshot. The selected
handle and its metadata are captured during setup, so replacing the pathname,
selected root, or a path component after open cannot redirect the response.
The `Content-Length` header is the length captured from that handle and is
never recomputed.

In-place writes may be observed by later reads. A same-length overwrite may
therefore yield old bytes, new bytes, or a mixture, but succeeds only if the
stream yields exactly the captured length. Truncation can yield a prefix then
fails with the stable `Io` provider category. Growth yields no bytes beyond
the captured length and likewise fails with `Io` after the bounded extra-byte
probe. Media type and other response metadata remain those captured during
setup. All success, mismatch, cancellation, and body-drop paths release the
opened handle, channel, and reader permit.

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
