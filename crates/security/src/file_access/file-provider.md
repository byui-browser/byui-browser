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
handles, with path-component containment and symlink/junction escape checks.
Root configuration accepts only a trusted canonical selection path paired with
its already-open directory handle, eliminating the prior policy-versus-handle
root-replacement race.

Authorized `GET` requests stream 64 KiB chunks through a bounded channel;
`HEAD` performs authorization and metadata lookup without starting a reader.
Four readers and eight pending opens are admitted, and cancellation releases
provider resources. The provider avoids HTTP transport, cookies, service
workers, and the HTTP cache, and exposes stable non-diagnostic error
categories. Browser and security tests cover the implemented authorization,
containment, streaming, cancellation, revocation, and failure behavior.

## Remaining work

### Trusted composition and permission lifetime

- Connect `TrustedFileAccessRoot::from_open_directory` to user-selected
  directory handles or an equivalent narrow platform capability before enabling
  local-file UI. Its canonical selection path and directory handle must be
  captured by the same trusted platform operation; this is what prevents a
  root-replacement race during configuration. Renderers and
  request-construction code must never receive source roots or the capability
  itself.
- Define permission lifetime and revocation behavior for tab/document close,
  navigation, process shutdown, grant expiry, and audit logging. Specify which
  already-open streams survive each transition.
- Keep navigation-authorization state private to trusted browser navigation
  code as new navigation entry points are added.

### File-document origins and request policy

The current policy permits only explicitly granted top-level navigations. Define
file-document origin identity, same-origin relationships, directory sharing,
cross-origin navigation behavior, and sandbox/container binding. Then decide
which scripts, workers, stylesheets, and other subresources may read local
files, while preserving fail-closed behavior.

### Limits, metadata, and content handling

- Define and enforce maximum file size, read duration, total bytes served, and
  aggregate blocking-reader/channel memory. The current admission bounds limit
  concurrency and buffering but do not bound the duration or total cost of a
  permitted read.
- Add filesystem media-type detection and decide whether invalid detected
  metadata fails the request or is omitted. Define MIME sniffing, `nosniff`, and
  executable-content policy before local HTML, script, image, or similar
  content is interpreted.
- Decide whether directories and other unsupported file types should retain the
  current non-diagnostic `Io` category or receive a dedicated category.

### Stream consistency and lifecycle coverage

- Keep `Content-Length` consistent with the live stream. The provider currently
  captures length before reading but does not enforce it if the file is
  truncated or extended during the read. Define snapshot consistency and either
  enforce the captured length or fail when the file changes.
- Define content consistency for mutable opened files. Retaining a handle
  prevents pathname replacement, but not in-place writes, truncation, or other
  mutations while streaming. Add detection or document the supported guarantee.
- Handle aborts during response preparation. The blocking setup task checks
  cancellation before opening, but not after opening and metadata lookup, so an
  abort can still produce a successful response, especially for `HEAD`. Add
  post-setup checks and tests for aborts during open and metadata lookup.
- Add regression tests for unread and partially read body drops, worker-side
  read failures, full-channel backpressure, and the four-reader concurrency
  limit.

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
