# Browser `file:` provider

The browser adapts security-owned file access to `net::FileProvider`. The
security crate owns path authorization and filesystem capabilities; the browser
owns request-context translation and explicit grants; `net` owns scheme
dispatch, response validation, and body behavior. The security crate does not
depend on `net`.

## Current implementation

### Authorization and composition

- `FileAccessPolicy` is default-deny and canonicalizes configured roots.
- `FileAccessPolicy::authorize_request` requires granted permission, the
  configured `FileSandboxId`, a `file:` target origin, and a top-level
  navigation request before a path is opened.
- `FileRequestContext` carries the initiating origin, requested origin,
  navigation/subresource kind, permission state, and sandbox identity. The
  initiating origin is preserved for the later file-document origin policy;
  the current policy denies subresource requests.
- `BrowserFileProvider` translates `net::Request` into that context through an
  explicit `FileAccessGrant`. `FileAccessConfiguration` makes policy,
  filesystem, permission, and sandbox state explicit when composing the
  browser request controller.
- If no file configuration is supplied, no `file:` provider is installed and
  requests fail with `MissingProvider` before filesystem access.

### Capability-based filesystem access

- `CapFileSystem` opens each configured root as a directory capability.
- `FileAccessPolicy::open_file` authorizes a canonical path and passes an
  `AuthorizedPath` to the filesystem backend.
- The backend opens the file once and returns an `OpenedFile`; reads use that
  retained handle rather than reopening the authorized pathname.
- Root containment uses path components, so similarly named sibling roots are
  distinct. Canonicalization rejects traversal and symlink escapes.
- Current tests cover regular files, denied siblings, directories, replacement
  races, Unix symlink escapes, and Windows junction escapes.

### Provider response behavior

After request authorization and file opening, `BrowserFileProvider` currently:

- materializes the opened file with `OpenedFile::read_all`;
- emits `Content-Length` and an optional filesystem-supplied `Content-Type`;
- returns status `200 OK` with an opaque response origin; and
- supplies the bytes to `net` as `SchemeBody::Bytes`.

The shared `net` scheme pipeline enforces `GET`/`HEAD`, nulls the body for
`HEAD`, validates response metadata, and maps failures to stable provider error
categories without exposing filesystem paths or OS diagnostics. Browser tests
cover allowed reads, default denial, missing files, directories, malformed
URLs, content length, `GET`/`HEAD`, denied permissions, sandbox mismatches, and
subresource rejection.

## Remaining work

### Asynchronous reads and streaming

`OpenedFile::read_all` performs blocking I/O and materializes the complete
file. Move reads behind a bounded blocking executor or an asynchronous
filesystem boundary, then stream large or unknown-size files through
`net::SchemeBody::Stream` while retaining the opened handle. Verify cleanup
when requests or bodies are aborted, fail, dropped unread, or only partially
consumed.

### File-document origins and request policy

The current policy intentionally permits only explicitly granted top-level
navigations. Define file-document origin semantics and then decide which
subresource requests are allowed. Connect that decision to initiating origins,
same-origin checks, sandbox/container identity, and navigation policy while
preserving fail-closed behavior.

### Metadata and resource limits

Complete handling and tests for invalid media types, unreadable files,
unsupported file types, large files, read duration, and concurrent reads.
Decide whether the provider should reject invalid filesystem media metadata or
omit it, rather than silently dropping invalid values during header creation.

### Platform hardening and conformance

Extend coverage for Unix mount points and hard links, Windows reparse-point
variants, drive-letter and UNC paths, network shares, removable media, and
capability-backend fallbacks. Any fallback must open once, revalidate that
handle, and read from that same handle; it must never re-authorize and reopen a
pathname.

Keep security, browser, and `net` integration tests aligned with the guarantees
that local file requests do not use HTTP transport, cookies, the HTTP cache, or
unrelated scheme providers.

## Recommended order

1. Move file reads to bounded asynchronous or blocking-executor-backed I/O.
2. Add retained-handle streaming and lifecycle/limit tests.
3. Finalize file-document origin and subresource authorization semantics.
4. Extend platform hardening and broader conformance coverage.
