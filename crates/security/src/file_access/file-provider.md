# File provider hardening plan

`BrowserFileProvider` is currently a compiling scaffold that adapts the
security crate's filesystem policy to `net::FileProvider`. The implementation
must remain fail-closed and must not move filesystem policy into `net`.

## 1. Harden path authorization

Expand `security::file_access::FileAccessPolicy` and its tests to cover:

- allowed roots and similarly named sibling directories;
- `..` traversal and absolute-path handling;
- symlinks that resolve outside an allowed root; and
- Windows drive-letter and UNC path behavior.

Keep the default policy deny-by-default. The approved path should remain an
opaque `AuthorizedPath` rather than a caller-constructed `PathBuf`.

## 2. Close the authorization/read TOCTOU gap

The current flow canonicalizes a path and then later reopens it with
`fs::read`. A file or symlink could change between those operations.

Replace this with one of the following, depending on the platform backend:

1. authorize and open the file as one operation, carrying the opened handle;
2. use platform-specific no-follow/open-beneath APIs; or
3. revalidate the opened file before reading it.

The chosen approach must prevent a path authorized inside the sandbox from
being replaced with a path outside it before bytes are read.

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

`LocalFileSystem::read` currently performs synchronous `fs::read`, and the
work happens before the provider future is returned. This can block the async
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

## Recommended order

1. Fix path and symlink authorization, including the TOCTOU design.
2. Add request/origin authorization.
3. Introduce asynchronous reads and streaming.
4. Harden metadata and error mapping.
5. Wire the provider into browser composition.
6. Complete the security, browser, and net conformance tests.
