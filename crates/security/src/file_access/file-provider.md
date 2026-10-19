# File provider hardening plan

`BrowserFileProvider` adapts the security crate's filesystem policy to
`net::FileProvider`. The implementation must remain fail-closed and must not
move filesystem policy into `net`.

## Completed work

### Security-owned handle boundary

`FileAccessPolicy::open_file` authorizes a requested path and returns an
`OpenedFile`; `OpenedFile::read_all` consumes that same handle. The browser
adapter uses this path, so it no longer authorizes a pathname and subsequently
reads it with `fs::read`.

`CapFileSystem` rejects non-regular files and resolves paths through opened
allowed-root capabilities rather than reopening authorized absolute paths.

### Policy coverage

`FileAccessPolicy` remains default-deny and keeps `AuthorizedPath` opaque. Its
tests now cover allowed roots and descendants, sibling-prefix rejection,
`..` traversal, absolute paths, and Unix symlinks that resolve outside an
allowed root. Windows-only tests also verify component-based drive-letter and
UNC containment comparisons.

### Generic `file:` provider support in `net`

The network engine dispatches `file:` requests only through an installed
provider, accepts only `GET` and `HEAD`, and does not invoke HTTP transport,
cookies, or the HTTP cache. It validates provider response metadata and has
tests for provider-future cancellation, stream cancellation, and unread or
partially consumed stream cleanup. `SchemeBody::Stream` is available when the
filesystem layer later adds streaming.

### Native containment backend and browser composition

`CapFileSystem` opens each configured root as a capability directory and
resolves authorized paths relative to that directory. This supplies the
multi-component containment needed to reject junction and symlink escapes,
including replacement races, while preserving the retained-handle boundary.
The browser crate exposes `request_controller`, which installs the file
provider only when explicit policy and filesystem configuration is supplied.

### Containment and adapter test coverage

Security tests now cover regular-file reads, default-deny behavior, directory
rejection, Unix symlink escapes, Windows junction escapes, and reads from the
original opened handle after a pathname replacement. Browser integration tests
cover explicit provider configuration, allowed reads, denied reads, missing
files, directories, malformed file URLs, content length, and `GET`/`HEAD`
behavior.

## Remaining work

### 1. Validate native containment across platforms

`CapFileSystem` is the production filesystem backend. It delegates the
platform-specific directory-capability and handle-opening details to
`cap-std`; this project does not need separate `UnixFileSystem` and
`WindowsFileSystem` implementations unless a future platform requirement is
not covered by that dependency.

The remaining work is platform-specific validation and regression coverage:

- On Unix, verify symlink, mount, and replacement-race behavior.
- On Windows, verify junction and reparse-point rejection, including
  drive-letter and UNC paths.
- Confirm that all platforms return an `OpenedFile` whose handle is retained
  for reading and that no caller reopens the authorized pathname.
- Keep any fallback fail-closed: it may open once, revalidate that exact
  handle, and read from that same handle. Revalidating a pathname and then
  reopening it is not acceptable.

Additional platform coverage is still needed for mount points, hard links,
network shares, and removable media before expanding the set of filesystem
features that can be configured as browser file roots.

### 2. Apply request and origin authorization

`BrowserFileProvider::open` currently ignores the `Request`. Add a security
request context that can evaluate:

- initiating origin;
- requested file origin;
- navigation versus subresource access;
- permission state; and
- browser sandbox or container identity.

The browser adapter should translate `net::Request` into that security-owned
context. The security crate should not depend on `net`.

### 3. Make browser file reads asynchronous and cancellable

`OpenedFile::read_all` currently performs synchronous I/O and materializes the
complete file before the provider future is returned. This can block the async
executor and prevents cancellation during the read.

Introduce an asynchronous filesystem boundary or move blocking operations to a
bounded `spawn_blocking` path. Verify that aborting a request releases any
opened file handle and pending task. The generic cancellation behavior in `net`
is already covered; this work must connect the browser filesystem operation to
that behavior.

### 4. Stream large files from the opened handle

`FileResource` currently materializes every file into `Vec<u8>`. Add a file
stream representation and map it to `net::SchemeBody::Stream` for large or
unknown-size files.

Test that handles and streams are released when:

- the request is aborted;
- the stream returns an error;
- the body is dropped before being read; and
- the body is only partially consumed.

### 5. Finish browser-adapter metadata and error mapping

The network response boundary rejects invalid or conflicting metadata, but
`BrowserFileProvider` currently drops an invalid filesystem-supplied media type
before the boundary can validate it. Invalid metadata must instead produce a
stable provider error.

Define behavior for:

- invalid media types;
- directories;
- unreadable files;
- unsupported file types; and
- content length and content type generation.

Keep provider errors mapped to the stable `net::ProviderErrorKind` categories
without exposing raw filesystem paths or operating-system error details.

### 6. Decide and document `file:` origin behavior

The current adapter assigns an opaque origin to every file response. Confirm
whether that is the intended browser policy, then document and test the choice.

The policy must be consistent with response validation, same-origin checks,
and any future request-context authorization.

### 7. Wire the provider into browser composition

Construct the provider explicitly from:

- a configured `FileAccessPolicy`;
- `CapFileSystem` configured with the same allowed roots; and
- `BrowserFileProvider`.

Install it with `SchemeProviders::with_file(...)` when creating the request
controller. No roots or policy should mean no file capability; missing lookup
must never fall back to HTTP or unrestricted filesystem access.

### 8. Extend security and browser integration coverage

The initial security and browser integration coverage is complete. Keep the
existing `net` provider-boundary tests and extend the concrete adapter tests as
new policy and metadata behavior is added.

The minimum matrix is:

- default-deny and allowed-root reads;
- traversal, sibling-prefix, symlink, junction, and replacement-race rejection
  (covered for the current capability backend);
- missing, denied, directory, unsupported-file, and I/O failures;
- valid and invalid media types (once filesystem media-type metadata is
  configurable);
- content length and response origin;
- `GET` and `HEAD` behavior;
- cancellation and body-drop cleanup; and
- no HTTP transport, cache, cookie, or unrelated-provider side effects.

## Future hardening

- Extend capability-root handling to additional platform-specific filesystem
  semantics where `cap-std` does not yet provide the required guarantee.
- Set explicit file-size, stream-duration, and concurrent-read limits to bound
  resource use by local content.
- Audit hard-link, mount-point, network-share, and removable-media semantics
  for each supported platform before broadening allowed-root configuration.
- Add fuzzing or property tests for path parsing and backend containment,
  especially Windows reparse-point and UNC cases.

## Recommended order

1. Expand platform-specific junction/reparse-point coverage.
2. Add request/origin authorization.
3. Introduce asynchronous reading and streaming from the retained handle.
4. Finish metadata, error mapping, origin policy, and conformance coverage.
