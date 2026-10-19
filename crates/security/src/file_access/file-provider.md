# File provider hardening plan

`BrowserFileProvider` adapts the security crate's filesystem policy to
`net::FileProvider`. Filesystem policy remains security-owned, and all access
must remain fail-closed.

## Completed

- `FileAccessPolicy` is default-deny and produces opaque authorized paths.
- `CapFileSystem` opens configured roots as capabilities and reads from the
  retained handle without reopening an authorized pathname.
- Browser composition installs `file:` access only with explicit policy and
  filesystem configuration.
- Security and browser tests cover allowed and denied reads, traversal and
  sibling rejection, symlink and junction escapes, replacement races, missing
  files, directories, malformed URLs, content length, and `GET`/`HEAD`.
- The generic `net` provider boundary covers provider routing, metadata
  validation, and provider/body cancellation behavior.

## Remaining work

### Request and origin authorization

`BrowserFileProvider::open` still ignores `Request`. Add a security-owned
request context covering:

- initiating and requested file origins;
- navigation versus subresource access;
- permission state; and
- browser sandbox or container identity.

Translate `net::Request` into that context in the browser adapter without
making the security crate depend on `net`.

### Asynchronous reads and streaming

`OpenedFile::read_all` performs blocking I/O and materializes the entire file.
Move reads to a bounded blocking executor or introduce an asynchronous
filesystem boundary. Then stream large or unknown-size files through
`net::SchemeBody::Stream` while retaining the opened handle.

Verify cleanup when requests or bodies are aborted, fail, dropped unread, or
are only partially consumed.

### Metadata, errors, and origin policy

Complete browser-adapter behavior for:

- valid and invalid filesystem media types;
- directories, unreadable files, and unsupported file types;
- content length and content type generation;
- stable provider error categories without leaking paths or OS details; and
- the chosen `file:` response origin policy.

Document and test the origin decision against response validation, same-origin
checks, and future request authorization.

### Platform and filesystem hardening

Extend validation of `CapFileSystem` for filesystem semantics not covered by
the current tests:

- Unix mount points and hard links;
- Windows reparse-point variants, drive-letter paths, and UNC paths;
- network shares and removable media; and
- fallback behavior when the capability backend lacks a required primitive.

Any fallback must open once, revalidate that exact handle, and read from that
same handle. Revalidating a pathname and then reopening it is not acceptable.

### Resource limits and broader coverage

Add explicit file-size, stream-duration, and concurrent-read limits. Extend
security, browser, and `net` integration coverage as these capabilities land,
including no HTTP transport, cache, cookie, or unrelated-provider side
effects.

## Recommended order

1. Add request and origin authorization.
2. Make reads asynchronous and cancellable.
3. Add retained-handle streaming.
4. Finalize metadata, errors, and origin behavior.
5. Extend platform hardening, resource limits, and conformance coverage.
