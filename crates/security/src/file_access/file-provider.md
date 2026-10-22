# Browser `file:` provider

The browser adapts security-owned file access to `net::FileProvider`. The
security crate owns path authorization and filesystem capabilities; the browser
owns request-context translation and explicit grants; `net` owns scheme
dispatch, response validation, and body behavior. The security crate does not
depend on `net`.

## Current implementation

### Authorization and composition

- `FileAccessCapability` is the security crate's trusted browser/platform
  composition boundary. It creates the default-deny policy and directory
  capabilities from the same approved roots, so request code cannot combine a
  policy for one root set with filesystem handles for another.
- `FileAccessCapability::open_authorized_file` requires a grant issued by that
  capability, a `file:` target origin, and a top-level navigation request
  before it canonicalizes or opens a path. The opaque grant binds
  authorization to one capability and therefore to its browser
  sandbox/container capability.
- `FileRequestContext` carries the initiating origin, requested origin,
  and navigation/subresource kind. The initiating origin is preserved for the
  later file-document origin policy; the current policy denies subresource
  requests.
- `BrowserFileProvider` translates `net::Request` into that context through an
  explicit `FileAccessGrantSource`. `FileAccessConfiguration` makes policy,
  filesystem, and shared policy-bound grant state explicit when composing the
  browser request controller.
- `FileAccessGrantSource` snapshots its state immediately before authorization.
  Revoking it denies future opens; already opened streams retain their existing
  capability until EOF, error, abort, or drop.
- A poisoned, empty, or revoked grant source fails closed. Replacing or
  revoking a grant only affects future opens.
- Browser controllers install `BrowserFetchServices`, allowing requests with a
  client environment to reach the provider. Its current policy is conservative:
  no cookies, no service-worker interception, and a process-local default cache
  partition.
- If no file configuration is supplied, no `file:` provider is installed and
  requests fail with `MissingProvider` before filesystem access.

### Capability-based filesystem access

- `CapFileSystem` opens each configured root as a directory capability.
- `FileAccessPolicy::open_authorized_file` atomically verifies the request
  context and policy-bound grant before canonicalizing the requested path or
  opening a file. Its capability-backed filesystem parameter and internal
  `AuthorizedPath` keep lower-level pathname opening out of the public API.
- The backend opens the file once and returns an `OpenedFile`; all reads use
  that retained handle rather than reopening the authorized pathname.
- Root containment uses path components, so similarly named sibling roots are
  distinct. Canonicalization rejects traversal and symlink escapes.
- Directory capabilities retain the root handle selected at configuration time.
  On platforms that permit a configured root pathname to be replaced, later
  requests continue to read from that retained root rather than a replacement
  pathname. A hard link named inside an allowed root is an allowed entry; path
  containment does not attempt inode-provenance checks.
- The current backend exposes no media type, so responses include
  `Content-Length` but no `Content-Type`. The adapter will include a media type
  only when a future backend supplies a valid header value.
- Tests cover regular files, denied siblings, directories, retained handles
  after Unix pathname and root replacement, Unix hard links and symlink
  escapes, and Windows junction escapes. A denied request for a missing path
  is also verified to return `AccessDenied` before path canonicalization can
  reveal that the path is missing.

### Provider response and body lifecycle

For an authorized `GET`, `BrowserFileProvider`:

- acquires one of four bounded file-read slots;
- performs authorization, opening, and metadata lookup on Tokio's blocking
  executor;
- returns the `200 OK` response metadata immediately, including
  `Content-Length` and an opaque response origin;
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
filesystem paths or OS diagnostics. Missing files map to `ResourceMissing`,
denied requests map to `AccessDenied`, malformed file paths map to
`MalformedMetadata`, and directories or filesystem I/O failures map to `Io`.
Browser tests cover allowed reads, default denial, missing files, directories,
malformed URLs, content length, live chunked bodies, `GET`/`HEAD`,
aborted-reader cleanup, revoked grants, foreign-policy grants, and subresource
rejection.

## Remaining work

### Streaming lifecycle coverage and limits

The current bounded-reader streaming contract is implemented. Add
browser-provider regression tests for unread and partially read body drops,
worker-side read failures, full-channel backpressure, and the four-reader
concurrency limit.

### File-document origins and request policy

The current policy intentionally permits only explicitly granted top-level
navigations. Define file-document origin semantics and then decide which
subresource requests are allowed. Connect that decision to initiating origins,
same-origin checks, sandbox/container identity, and navigation policy while
preserving fail-closed behavior.

### Metadata and resource limits

Add filesystem media-type detection and decide whether an invalid detected
value should fail the request or be omitted. Define and enforce product limits
for total file size, read duration, and acceptable chunk/channel memory.
Consider whether directories and other unsupported file types should retain the
current non-diagnostic `Io` category or receive a dedicated provider category.

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

1. **Keep root configuration inside trusted browser code.** `FileAccessCapability`
   is now the only public composition path and creates policy and filesystem
   state together. Wire its `from_trusted_roots` call to user-selected handles
   or an equivalent narrow platform capability before enabling local-file UI;
   renderers and request-construction code must never receive source roots or
   the capability itself.
2. **Define file-document origin and subresource semantics before enabling
   subresources.** The current top-level-navigation-only rule is fail-closed,
   but the initiating origin is not yet used to decide access. Specify file
   document origin identity, same-origin relationships, directory sharing,
   cross-origin navigation behavior, and sandbox/container binding before
   allowing scripts, workers, stylesheets, or other subresources to read local
   files.
3. **Complete platform containment hardening.** Authorization canonicalizes a
   path before the capability backend opens its relative path. The Unix
   root-replacement and hard-link cases are covered, as are Windows
   drive-letter, UNC, and junction component cases. Preserve the
   single-open/retained-handle invariant and add conformance tests for Unix
   mount changes, Windows reparse-point variants, network shares, removable
   media, and any remaining root-replacement races. Any fallback must fail
   closed rather than re-authorizing and reopening a pathname.
4. **Add file resource limits.** Define and enforce maximum file size, read
   duration, total bytes served, and aggregate blocking-reader/channel memory.
   The current four-reader semaphore and bounded channel limit concurrency and
   buffering, but they do not prevent a permitted path from consuming
   resources for an unbounded duration.
5. **Define safe file metadata and content handling.** Add filesystem media-type
   detection and decide whether invalid metadata fails the request or is
   omitted. Before local HTML, script, image, or similar content is interpreted,
   define MIME sniffing, `nosniff`, and executable-content policy so a local
   file cannot acquire an unintended active-content interpretation.
6. **Define permission lifetime and revocation semantics.** Revocation currently
   blocks future opens while already-open streams retain their handles until
   completion, failure, abort, or drop. Specify behavior for tab/document
   close, navigation, process shutdown, user-grant expiry, and audit logging;
   add tests for each lifecycle transition.
7. **Bind navigation authorization to trusted browser state.** The provider
   currently maps the public `RequestMode::Navigate` value directly to
   `FileRequestKind::Navigation`. Because request construction can set that
   value, a caller that can reach the controller may be able to turn an
   ordinary file request into an apparently top-level navigation and bypass
   the subresource restriction. Require a browser-issued navigation context or
   equivalent capability rather than trusting the request mode alone, and add
   a regression test proving that a caller-controlled mode cannot authorize
   local-file access.
8. **Bound pending provider work and make permit acquisition cancellable.**
   The four-reader semaphore limits active readers, but requests waiting for a
   permit are not bounded and the wait does not observe the request abort
   signal. A caller can therefore accumulate abandoned pending requests and
   delay cancellation while active readers remain occupied. Add a bounded
   pending-request policy and cancellation-aware permit acquisition, with
   tests for aborts before a permit is available and for queue exhaustion.
9. **Normalize pre-authorization path errors.** URL-to-filesystem-path
   conversion currently occurs before grant and request-context authorization,
   allowing unauthorized callers to distinguish malformed file URLs from
   ordinary denied requests. Perform authorization before filesystem path
   conversion where possible, or map all such pre-authorization failures to a
   uniform access-denied result; add a regression test for the error behavior.

## Recommended order

1. Keep root configuration in trusted browser composition and complete
   platform containment hardening.
2. Finalize file-document origin and subresource authorization semantics.
3. Add resource limits, safe metadata/content handling, and permission-lifetime
   coverage.
4. Add streaming lifecycle, concurrency-limit, and read-failure coverage.
