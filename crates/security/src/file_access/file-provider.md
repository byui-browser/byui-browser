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

Grants may have a browser-configured lifetime. Expiry, replacement, explicit
revocation, and shutdown all cancel pending setup and live or queued body
delivery through the same revocation path, clear unused navigation
authorizations, and record path-free lifecycle audit events. After expiry,
future navigations are denied and existing bodies report cancellation.

The provider also bounds aggregate queued response bytes per session and wakes
body consumers at their read deadline, cancelling further delivery. A regular
filesystem read already executing in the safe blocking API cannot be forcibly
interrupted; its result is discarded and all provider resources are released
as soon as that operation returns.

The default limits are a 64 MiB maximum file size, 256 MiB of response bytes
per session, a 30-second setup-and-read deadline, and 512 KiB of aggregate
queued body data. At most four file readers are active and eight additional
authorized requests may wait for admission. These browser-owned limits can be
made stricter through configuration; zero-valued limits are rejected.

Each admitted `file:` document receives an opaque origin. Local-file access is
limited to browser-issued top-level navigations; all subresources, workers, and
nested browsing contexts remain denied until document/container identity is
available end-to-end.

Callers must create navigations with
`FileAccessConfiguration::navigation_request`. Setting
`RequestMode::Navigate` on an ordinary request does not issue the required
single-use authorization. At most 64 unused navigation authorizations may be
held at once. `GET` and `HEAD` are supported; `HEAD` performs the same
authorization and content checks but returns no body and consumes no session
response-byte budget.

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
These decisions are part of the browser contract and are enforced before
response metadata or body data is exposed.

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
- Unknown extensions are not admitted by the current static policy. They must
  never receive an inferred active media type; serving inert binary data would
  require a separate consumer path with an explicit policy.
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

## Regression and integration guarantees

Deterministic regression coverage exercises blocking setup, response
publication, queued chunks, worker-side failures, full-channel backpressure,
four-reader concurrency, queue exhaustion, request aborts, and `GET`/`HEAD`
revocation behavior. Each completion, failure, cancellation, and body-drop
path releases its handles, permits, channels, and blocking tasks.

Cross-crate tests verify that local-file requests bypass HTTP transport,
cookies, service workers, and the HTTP cache; unrelated providers cannot
satisfy a `file:` request; provider errors remain stable and non-diagnostic;
and unauthorized requests are rejected before path conversion.

There is no automatic `Drop`-based session shutdown. Browser lifecycle code
must explicitly call `close`, `navigation_away`, `shutdown`, or revoke the
grant, unless a grant lifetime is configured. The audit stream contains only
the path-free events `Replaced`, `Revoked`, `Expired`, and `Shutdown`.

## Potential future security work

The following risks are known and intentionally remain for now. They must not
be treated as guarantees supplied by the file provider alone.

- **Active HTML content:** `text/html` is currently admitted as a static
  document type, but the provider does not itself install a CSP, sandbox
  policy, or HTML/script filtering. A future renderer integration must ensure
  that scripts, inline event handlers, active navigation, and other HTML
  capabilities cannot execute or escape the intended local-file policy. The
  existing opaque origin and `nosniff` header are not sufficient by themselves.
- **Outbound network policy:** the file provider blocks local-file
  subresources, but it does not by itself prevent a future document or renderer
  from issuing HTTP(S) requests. A browser-owned request policy must enforce the
  documented no-external-network default for file documents, including markup,
  script, form, popup, and navigation paths.
- **Content integrity after open:** retained handles prevent pathname
  replacement and symlink redirection, but they do not snapshot file bytes.
  Another process may modify the file in place after metadata and the bounded
  executable-prefix check have completed. Future policy should decide whether
  local documents require immutable snapshots, stronger file identity checks, or
  an explicit acceptance of this mutation behavior.
- **Lifecycle resource bounds:** audit events are retained in memory without a
  configured cap, and each timed grant generation currently uses a sleeping
  background thread. Future work should bound audit retention and replace
  per-generation sleeping threads with a cancellable, bounded scheduler.
- **Windows alias timing:** the Windows long-name, short-name, and
  extended-length spelling fallback may consult the filesystem through
  `canonicalize`. It preserves stable denial categories, but future work should
  assess and, if necessary, remove filesystem-dependent timing differences when
  proving the no-existence-oracle property for alias spellings.

The security layer also fails closed for malformed or unauthorized paths before
filesystem conversion, avoids existence oracles for paths outside selected
roots, rejects symlink components, and reads through retained handles rather
than reopening a pathname. A file opened for a response remains bound to that
handle even if its directory entry is replaced.
