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
  explicit `FileAccessGrantSource`. `FileAccessConfiguration` makes policy,
  filesystem, and shared permission/sandbox state explicit when composing the
  browser request controller.
- `FileAccessGrantSource` snapshots its state immediately before authorization.
  Revoking it denies future opens; already opened streams retain their existing
  capability until EOF, error, abort, or drop.
- Browser controllers install `BrowserFetchServices`, allowing requests with a
  client environment to reach the provider. Its current policy is conservative:
  no cookies, no service-worker interception, and a process-local default cache
  partition.
- If no file configuration is supplied, no `file:` provider is installed and
  requests fail with `MissingProvider` before filesystem access.

### Capability-based filesystem access

- `CapFileSystem` opens each configured root as a directory capability.
- `FileAccessPolicy::open_file` authorizes a canonical path and passes an
  `AuthorizedPath` to the filesystem backend.
- The backend opens the file once and returns an `OpenedFile`; all reads use
  that retained handle rather than reopening the authorized pathname.
- Root containment uses path components, so similarly named sibling roots are
  distinct. Canonicalization rejects traversal and symlink escapes.
- Current tests cover regular files, denied siblings, directories, replacement
  races, Unix symlink escapes, and Windows junction escapes.

### Provider response and body lifecycle

For an authorized `GET`, `BrowserFileProvider`:

- acquires one of four bounded file-read slots;
- performs authorization, opening, and metadata lookup on Tokio's blocking
  executor;
- returns the `200 OK` response metadata immediately, including
  `Content-Length`, optional filesystem-supplied `Content-Type`, and an opaque
  response origin;
- moves the opened handle and read-slot permit into a separate blocking reader;
- streams 64 KiB chunks through a bounded two-chunk channel as
  `SchemeBody::Stream`; and
- releases the handle and permit at EOF, on a read error, on request abort, or
  when the caller drops the response body.

The response body is a one-shot live stream; the complete file is never
materialized in memory. The worker checks the request abort signal before each
read and while waiting for channel capacity. Dropping the resolution future
cancels setup; dropping the body closes the channel receiver and signals the
worker to stop. An operating-system read already in progress completes before
cancellation can take effect.

`HEAD` performs authorization, opening, and metadata lookup but starts no body
reader and returns `SchemeBody::Null`.

The shared `net` pipeline enforces `GET`/`HEAD`, validates response metadata,
and maps failures to stable provider error categories without exposing
filesystem paths or OS diagnostics. Browser tests cover allowed reads, default
denial, missing files, directories, malformed URLs, content length, live
chunked bodies, `GET`/`HEAD`, aborted-reader cleanup, denied permissions,
sandbox mismatches, and subresource rejection.

## Remaining work

### Streaming lifecycle coverage and limits

The current bounded-reader streaming contract is implemented. Add
browser-provider regression tests for unread and partially read body drops,
worker-side read failures, full-channel backpressure, and the four-reader
concurrency limit. Define and enforce product limits for total file size, read
duration, and acceptable chunk/channel memory.

### File-document origins and request policy

The current policy intentionally permits only explicitly granted top-level
navigations. Define file-document origin semantics and then decide which
subresource requests are allowed. Connect that decision to initiating origins,
same-origin checks, sandbox/container identity, and navigation policy while
preserving fail-closed behavior.

### Metadata and resource limits

Decide whether invalid filesystem media metadata should fail the request or be
omitted, rather than silently dropping an invalid `Content-Type` during header
construction. Complete handling for unreadable files and unsupported file
types under the streaming path.

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

The following issues remain before treating the provider as a complete browser
capability:

1. **Make grants and sandbox identity trustworthy.** `Granted` permission and
   `FileSandboxId` values are currently constructible by callers. Define the
   trust boundary for `FileAccessGrant` and sandbox identities, or replace
   forgeable values with browser-owned capabilities or tokens that untrusted
   code cannot manufacture.
2. **Prevent request-policy bypass through lower-level APIs.**
   `FileAccessPolicy::open_file` performs path authorization without requiring
   a `FileRequestContext`. Either enforce request authorization in the API
   boundary or make the lower-level path-opening API explicitly private to the
   browser adapter and document the invariant.
3. **Harden the `FileSystem` backend contract.** A custom `FileSystem`
   implementation can ignore the supplied `AuthorizedPath` and open an
   unrelated ambient path. Restrict implementations, strengthen the capability
   type, or add validation/tests so the security boundary does not rely only on
   an unenforced implementation contract.

## Recommended order

1. Close request-policy and filesystem-backend bypasses.
2. Add streaming lifecycle, concurrency-limit, and read-failure coverage.
3. Finalize file-document origin and subresource authorization semantics.
4. Extend platform hardening and broader conformance coverage.
