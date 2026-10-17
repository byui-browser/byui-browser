# File provider hardening plan

`BrowserFileProvider` adapts the security crate's filesystem policy to
`net::FileProvider`. The implementation must remain fail-closed and must not
move filesystem policy into `net`.

## Current baseline

The module now has an `OpenedFile` capability. Callers use
`FileAccessPolicy::open_file` to authorize a requested path and obtain an
opened handle, then read from that handle with `OpenedFile::read_all`.
`BrowserFileProvider` follows this path and no longer calls `fs::read` on an
already-authorized pathname.

`LocalFileSystem` remains a deliberately partial backend. It retains the
handle it opens and rejects non-regular files where the platform permits
inspection, but `std::fs::File::open` cannot constrain resolution of every path
component. It is not sufficient sandbox containment for untrusted paths.

## 1. Harden path authorization

Expand `security::file_access::FileAccessPolicy` and its tests to cover:

- allowed roots and similarly named sibling directories;
- `..` traversal and absolute-path handling;
- symlinks that resolve outside an allowed root; and
- Windows drive-letter and UNC path behavior.

Keep the default policy deny-by-default. The approved path should remain an
opaque `AuthorizedPath` rather than a caller-constructed `PathBuf`.

## 2. Implement native path-containment backends

The retained-handle model prevents a second pathname lookup after opening, but
the standard-library backend cannot atomically constrain path resolution.
Replace it in production with platform backends that enforce containment while
opening the file:

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

## 3. Apply request and origin authorization

`BrowserFileProvider::open` currently ignores the `Request`. Add a security
request context that can evaluate:

- initiating origin;
- requested file origin;
- navigation versus subresource access;
- permission state; and
- browser sandbox or container identity.

The browser adapter should translate `net::Request` into that security-owned
context. The security crate should not depend on `net`.

## 4. Make reads asynchronous and cancellable

`OpenedFile::read_all` currently performs synchronous I/O and materializes the
complete file before the provider future is returned. This can block the async
executor and prevents cancellation during the read.

Introduce an asynchronous filesystem boundary or move blocking operations to a
bounded `spawn_blocking` path. Verify that aborting a request releases any
opened file handle and pending task.

## 5. Stream large files

`FileResource` currently materializes every file into `Vec<u8>`. Add a file
stream representation and map it to `net::SchemeBody::Stream` for large or
unknown-size files.

Test that handles and streams are released when:

- the request is aborted;
- the stream returns an error;
- the body is dropped before being read; and
- the body is only partially consumed.

## 6. Validate response metadata

The adapter currently drops invalid media types silently. Invalid metadata
must produce a stable provider error instead.

Define behavior for:

- invalid media types;
- directories;
- unreadable files;
- unsupported file types; and
- content length and content type generation.

Keep provider errors mapped to the stable `net::ProviderErrorKind` categories
without exposing raw filesystem paths or operating-system error details.

## 7. Decide and document `file:` origin behavior

The current adapter assigns an opaque origin to every file response. Confirm
whether that is the intended browser policy, then document and test the choice.

The policy must be consistent with response validation, same-origin checks,
and any future request-context authorization.

## 8. Wire the provider into browser composition

Construct the provider explicitly from:

- a configured `FileAccessPolicy`;
- the platform filesystem backend; and
- `BrowserFileProvider`.

Install it with `SchemeProviders::with_file(...)` when creating the request
controller. No roots or policy should mean no file capability; missing lookup
must never fall back to HTTP or unrestricted filesystem access.

## 9. Add conformance and integration tests

Add security tests for authorization and path handling, browser tests for
adapter behavior, and net integration tests for the provider boundary.

The minimum matrix is:

- default-deny and allowed-root reads;
- traversal, sibling-prefix, and symlink escape rejection;
- missing, denied, directory, and I/O failures;
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

1. Implement and validate native path-containment backends.
2. Complete path-authorization and race-regression coverage.
3. Add request/origin authorization.
4. Introduce asynchronous reading and streaming.
5. Harden metadata and error mapping.
6. Wire the provider into browser composition and complete conformance tests.
