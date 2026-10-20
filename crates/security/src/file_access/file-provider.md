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

After request authorization and file opening, `BrowserFileProvider` now:

- acquires one of four bounded file-read slots;
- performs authorization, opening, and chunked reads on Tokio's blocking
  executor while retaining the opened handle;
- returns response metadata before body reads complete;
- sends 64 KiB chunks through a bounded two-chunk channel as
  `SchemeBody::Stream`;
- keeps its file-read slot and opened handle until the stream reaches EOF,
  fails, is aborted, or is dropped; and
- stops between chunks when the provider future or body stream is cancelled or
  dropped;
- emits `Content-Length` and an optional filesystem-supplied `Content-Type`;
- avoids reading file contents for `HEAD`, while still reporting metadata;
- returns status `200 OK` with an opaque response origin; and
- supplies a live body to `net` as `SchemeBody::Stream`.

The shared `net` scheme pipeline enforces `GET`/`HEAD`, nulls the body for
`HEAD`, validates response metadata, and maps failures to stable provider error
categories without exposing filesystem paths or OS diagnostics. Browser tests
cover allowed reads, default denial, missing files, directories, malformed
URLs, content length, live chunked bodies, `GET`/`HEAD`, aborted-reader
cleanup, denied permissions, sandbox mismatches, and subresource rejection.

## Remaining work

### Asynchronous reads and streaming

Authorization and opening occur on the bounded blocking executor before the
provider returns metadata. The opened handle then moves into a separate
bounded blocking reader, which streams chunks through `net::SchemeBody::Stream`
without materializing the full file. Verify cleanup when requests or bodies
are aborted, fail, dropped unread, or only partially consumed.

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

### Additional required work from implementation review

The following issues are not sufficiently covered by the work above and must
be addressed before treating the provider as a complete browser capability:

1. **Compose browser services for context-bearing requests.** The browser
   controller currently installs the file provider without `FetchServices`.
   File requests carrying an initiating environment, destination, or other
   browser context are rejected by `net` before reaching the provider. Add the
   browser service composition and test navigation requests with real
   initiating origins.
2. **Support permission revocation.** A `FileAccessGrant` is copied into the
   provider at controller construction and cannot be revoked or refreshed.
   Add a browser-owned permission source, revocation path, or capability
   generation check so an old controller cannot authorize access after a grant
   is withdrawn.
3. **Make grants and sandbox identity trustworthy.** `Granted` permission and
   `FileSandboxId` values are currently constructible by callers. Define the
   trust boundary for `FileAccessGrant` and sandbox identities, or replace
   forgeable values with browser-owned capabilities/tokens that untrusted code
   cannot manufacture.
4. **Prevent request-policy bypass through lower-level APIs.**
   `FileAccessPolicy::open_file` performs path authorization without requiring
   a `FileRequestContext`. Either enforce request authorization in the API
   boundary or make the lower-level path-opening API explicitly private to the
   browser adapter and document the invariant.
5. **Harden the `FileSystem` backend contract.** A custom `FileSystem`
   implementation can ignore the supplied `AuthorizedPath` and open an
   unrelated ambient path. Restrict implementations, strengthen the
   capability type, or add validation/tests so the security boundary does not
   rely only on an unenforced implementation contract.
6. **Make provider work genuinely abortable.** Authorization, opening, and
   `read_all` currently run synchronously before the provider future is
   returned. Move them off the async executor’s worker thread and make aborts
   interrupt or promptly stop in-progress reads.
7. **Avoid reading file contents for `HEAD`.** The provider currently reads and
   allocates the complete file even though `net` later exposes a null body for
   `HEAD`. Return metadata without materializing contents and add a regression
   test proving that `HEAD` does not perform a full read.
8. **Expose response headers before body completion.** `Content-Length` is
   currently computed only after `read_all` finishes, so callers cannot receive
   file response headers before the complete file is read. Separate metadata
   acquisition from body delivery and test response-before-body behavior for
   files.

## Recommended order

1. Move file reads to bounded asynchronous or blocking-executor-backed I/O.
2. Compose browser services and design revocable, trustworthy grants.
3. Close request-policy and filesystem-backend bypasses.
4. Add concurrency-limit and read-failure lifecycle coverage.
5. Finalize file-document origin and subresource authorization semantics.
6. Extend platform hardening and broader conformance coverage.
