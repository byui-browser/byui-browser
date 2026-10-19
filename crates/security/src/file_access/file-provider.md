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
partially consumed stream cleanup. `SchemeBody::Stream` is therefore available
to a future filesystem backend.

### Native containment backend and browser composition

`CapFileSystem` opens each configured root as a capability directory and
resolves authorized paths relative to that directory. This supplies the
multi-component containment needed to reject junction and symlink escapes,
including replacement races, while preserving the retained-handle boundary.
The browser crate exposes `request_controller`, which installs the file
provider only when explicit policy and filesystem configuration is supplied.

## Remaining work

### 1. Expand native containment coverage

The capability backend is now used for production-configured browser file
access. Continue platform-specific validation while broadening support:

- On Unix, anchor resolution at an opened allowed-root directory and use the
  strongest available no-follow or open-beneath primitive for every component.
- On Windows, open through handles that reject reparse-point and junction
  escapes, including drive-letter and UNC paths.
- Keep these details behind `FileSystem`; callers continue to receive only an
  `OpenedFile` and must never reopen an authorized pathname.
- Define a fail-closed fallback only when it opens once, revalidates that exact
  handle, and reads from that same handle. Revalidating a pathname and then
  reopening it is not acceptable.

Add platform-specific regression tests for symlink, junction, and replacement
races before enabling each backend for browser `file:` access.

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
- the platform filesystem backend; and
- `BrowserFileProvider`.

Install it with `SchemeProviders::with_file(...)` when creating the request
controller. No roots or policy should mean no file capability; missing lookup
must never fall back to HTTP or unrestricted filesystem access.

### 8. Add security and browser integration coverage

Keep the existing `net` provider-boundary tests, then add security tests for
the native backends and browser tests for the concrete adapter. There are not
yet browser-level tests that exercise `BrowserFileProvider` with a real policy
and filesystem backend.

The minimum matrix is:

- default-deny and allowed-root reads;
- traversal, sibling-prefix, symlink, junction, and replacement-race rejection;
- missing, denied, directory, unsupported-file, and I/O failures;
- valid and invalid media types;
- content length and response origin;
- `GET` and `HEAD` behavior;
- cancellation and body-drop cleanup; and
- no HTTP transport, cache, cookie, or unrelated-provider side effects.

## Future hardening

- Retain allowed roots as directory capabilities rather than canonicalized
  paths where platform support permits it.
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
