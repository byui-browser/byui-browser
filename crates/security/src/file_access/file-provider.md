# Browser `file:` provider

The browser adapts security-owned file access to `net::FileProvider`. The
security crate owns path authorization and filesystem capabilities; the browser
owns request-context translation and explicit grants; `net` owns scheme
dispatch, response validation, and body behavior. The security crate does not
depend on `net`.

## Completed work

The current implementation provides a default-deny, capability-backed provider
for explicitly configured roots. Browser-issued, single-use navigation
authorizations and policy-bound grants are required before URL-to-path
conversion. Requests are opened once through retained directory and file
handles. Every component is opened relative to the retained root with symlink
following disabled, so a component replacement cannot redirect the final open
outside the selected tree.
Root configuration is intended to accept only a trusted canonical selection
path paired with its already-open directory handle, eliminating the prior
policy-versus-handle root-replacement race when the trusted boundary is used
correctly.

Authorized `GET` requests stream 64 KiB chunks through a bounded channel;
`HEAD` performs authorization and metadata lookup without starting a reader.
Four readers and eight pending opens are admitted, and cancellation releases
provider resources, including when cancellation arrives during response
preparation. Revocation, navigation-away, and session close deny future opens,
clear pending navigations, and cancel setup plus live response bodies. The
browser admits files only up to its configured per-file size limit (64 MiB by
default). A response body must yield exactly the length captured for its
`Content-Length` header: early EOF and trailing bytes fail the body with the
non-diagnostic `Io` category.
An allowlisted, case-insensitive filename extension determines the media type
for HTML, CSS, JavaScript, JSON, and common image formats. Unknown extensions
omit `Content-Type`; the provider does not sniff file contents.
Directories and other non-regular files fail with the stable, non-diagnostic
`UnsupportedResource` category rather than being reported as generic I/O.
The provider avoids HTTP transport, cookies, service
workers, and the HTTP cache, and exposes stable non-diagnostic error
categories. Browser and security tests cover the implemented authorization,
containment, streaming, cancellation, unread and partially read body drops,
revocation, and failure behavior.

## Remaining work

### Trusted composition and permission lifetime

- Connect `TrustedFileAccessRoot::from_open_directory` to user-selected
  directory handles or an equivalent narrow platform capability before enabling
  local-file UI. Its canonical selection path and directory handle must be
  captured by the same trusted platform operation; this is what prevents a
  root-replacement race during configuration. Renderers and
  request-construction code must never receive source roots or the capability
  itself.
- Remove the public path/handle pairing footgun: make root construction
  platform-private or expose only an opaque value produced by the trusted
  selection operation. Add a regression test or type-level boundary showing
  that a canonical path from one directory cannot be paired with another
  directory handle.
- Prevent authorized callers from using path errors as an outside-root
  existence oracle. Establish lexical/root eligibility before filesystem
  canonicalization where possible, or normalize missing and other lookup
  failures for paths outside the configured roots to `AccessDenied`. Test both
  existing and missing outside-root paths, including malformed and symlinked
  paths, without exposing filesystem details.
- Make revocation linearizable with authorization and open. Carry a session
  generation or revocation token into the security-owned open operation, reject
  it if the session changes before the handle is admitted, and ensure response
  construction cannot publish a `HEAD` response or a queued body chunk after
  revocation. Add race-focused tests for revocation before open, during setup,
  immediately before response construction, and while a body chunk is queued.
- Add grant expiry and audit logging. Process shutdown relies on process
  teardown today; define any explicit shutdown sequencing required by future
  browser lifecycle code.
- Keep navigation-authorization state private to trusted browser navigation
  code as new navigation entry points are added.

### File-document origins and request policy

The current policy permits only explicitly granted top-level navigations. Define
file-document origin identity, same-origin relationships, directory sharing,
cross-origin navigation behavior, and sandbox/container binding. Then decide
which scripts, workers, stylesheets, and other subresources may read local
files, while preserving fail-closed behavior.

### Limits, metadata, and content handling

- Define and enforce read duration, total bytes served, and aggregate
  blocking-reader/channel memory. The provider enforces a configurable
  per-file size limit and admission bounds concurrency and buffering, but it
  does not yet bound the duration or total cost of a permitted read.
- Decide whether MIME sniffing, `nosniff`, or a stricter executable-content
  policy is needed before local HTML, script, image, or similar content is
  interpreted. The current extension allowlist omits unknown types and does
  not inspect file contents.

### Stream consistency and lifecycle coverage

- Define content consistency for mutable opened files. Retaining a handle
  prevents pathname replacement, but not in-place writes or other mutations
  while streaming. The provider detects truncation and extension that would
  violate `Content-Length`, but does not provide a byte-for-byte snapshot when
  content changes without changing its length. Add detection or document the
  supported guarantee.
- Add regression tests for worker-side read failures, full-channel
  backpressure, and the four-reader concurrency limit.

### Platform hardening and capability construction

- Extend conformance coverage for Unix mount changes, Windows reparse-point
  variants, drive-letter and UNC paths, network shares, removable media, and
  remaining root-replacement races.
- Preserve the single-open/retained-handle invariant in every backend or
  fallback. A fallback must fail closed rather than re-authorize and reopen a
  pathname.
- Keep every platform implementation on the already-open-handle construction
  path. Do not add a convenience constructor that canonicalizes and reopens a
  root pathname, because it would reintroduce the root-replacement race closed
  by `TrustedFileAccessRoot`.

### Integration and conformance

Keep security, browser, and `net` integration tests aligned with the guarantees
that local-file requests do not use HTTP transport, cookies, the HTTP cache, or
unrelated scheme providers.

## Recommended order

1. Keep root configuration in trusted browser composition and complete platform
   containment hardening.
2. Finalize file-document origin and subresource authorization semantics.
3. Add resource limits, safe metadata/content handling, and permission-lifetime
   coverage.
4. Complete stream-consistency, setup-cancellation, and lifecycle coverage.
