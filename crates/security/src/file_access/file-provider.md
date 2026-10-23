# Browser `file:` provider

The browser adapts security-owned file access to `net::FileProvider`. The
security crate owns path authorization and filesystem capabilities; the browser
owns request-context translation, grants, limits, and session lifecycle; `net`
owns scheme dispatch, response validation, and body behavior.

## Current status

The provider is a default-deny, capability-backed implementation for trusted
directory selections. It requires browser-issued single-use navigation
authorization, opens files through retained handles with symlink following
disabled, streams bounded 64 KiB chunks, limits admission to four readers and
eight pending opens, and supports request abort plus session revocation.

The current slice also provides a configurable per-file size limit, allowlisted
extension-based media types, exact `Content-Length` enforcement, and
close/navigation-away cancellation. Core authorization, containment,
streaming, admission, lifecycle, media-type, size-limit, and length-consistency
tests pass. Path eligibility now precedes filesystem resolution, so ineligible
existing and missing paths receive the same denial. The remaining work below is
limited to provider behavior, policy, lifecycle, and regression coverage.

Trusted directory composition now verifies that the supplied canonical path and
the retained `Dir` identify the same directory before a selection is accepted.
This prevents configuration from authorizing paths below one directory and then
reading identically named files through another directory's handle. After this
one-time verification, all opens remain relative to the retained handle, so a
replacement of the pathname cannot redirect reads.

The current crate does not provide a native directory picker. The public
selection constructor is therefore treated as a trusted integration boundary;
renderer, web-content, and ordinary request-construction code must not receive
its inputs, a selection, or a `FileAccessCapability`.

## Remaining work

### 1. Finish trusted-composition regression coverage

The implemented `from_verified_open_directory` constructor rejects a path and
handle that identify different filesystem objects. Regression tests cover that
mismatch, replacement between handle capture and configuration, and a matched
retained handle continuing to serve the original directory after its pathname
is replaced.

Keep the path/handle identity check, retained-handle behavior, and root
replacement tests as part of the provider contract. Add current-scope tests for
malformed absolute paths, traversal, existing and missing symlink escapes, and
the stable denial behavior of the Windows alias fallback. Do not add a fallback
that reauthorizes or reopens the root pathname.

### 2. Make revocation fully linearizable

Session grants now own a generation-bound security lease. The browser carries
that lease into the security-owned open operation, which validates it before
filesystem work and before returning the opened handle. The browser validates
it once more at response publication and before delivering every body chunk.
Revocation and validation serialize through the session state: an open that
observes revocation before its publication check is aborted without exposing
metadata or a body.

An already-running OS read may finish internally after revocation, but its
result is discarded. A chunk already queued in the bounded channel is also
checked by the body stream before it reaches Fetch. `GET` and `HEAD` share the
same final publication validation. Close, navigation-away, and grant
replacement all revoke the prior grant; future expiry support must use this
same grant-revocation path.

Regression coverage includes post-setup revocation for `GET` and `HEAD`,
admission release, and revocation after a chunk has been queued. Follow-up
tests should retain deterministic hooks for blocking setup and the exact
publication handoff when those paths gain additional asynchronous work.

### 3. Define file-document origins and subresource policy

The current policy permits only explicitly granted top-level navigations, and
the initiating origin is carried through but not used to authorize access.

Define the origin assigned to a file document and its relationship to:

- another file document in the same directory or root;
- a document from `http:` or `https:`;
- opaque and sandboxed origins;
- workers and nested browsing contexts; and
- navigation between files and directories.

Use those rules to decide whether scripts, workers, stylesheets, images, and
other subresources may read local files. Connect the decision to same-origin
checks, sandbox/container identity, navigation policy, grants, and request
context. Keep unsupported cases denied until the full policy is implemented.

### 4. Complete resource and content limits

Per-file size limits, bounded active/pending admission, bounded chunking, and
allowlisted extension-based media types are implemented. The per-file limit
does not yet bound the lifetime or aggregate cost of a session.

Define and enforce:

- maximum read duration;
- total bytes served per session;
- aggregate blocking-reader and channel memory; and
- any additional platform-specific I/O limits.

Define MIME sniffing, `nosniff`, and executable-content rules before local HTML,
JavaScript, images, stylesheets, or other active content is interpreted. Decide
whether directories and unsupported file types retain the current non-diagnostic
`Io` category or receive dedicated errors.

Test every limit at its boundary, including slow reads, concurrent readers,
queue exhaustion, and cancellation while a limit is being reached.

### 5. Define mutable-file and stream consistency semantics

The provider now enforces the captured `Content-Length`: early EOF and extra
bytes produce a provider failure. It does not yet define whether local-file
responses are snapshot-consistent or may observe mutable file contents.

Choose whether local-file responses require a stable snapshot or support live
file contents. Document the existing length-mismatch failure behavior and define
how to handle in-place writes, truncation, growth, and metadata changes after
authorization. If live contents are supported, define response framing and its
security implications explicitly.

Add mutation tests beyond the existing length-mismatch coverage, including
changes during setup and streaming, and verify cleanup of the retained handle,
channel, and read permit.

### 6. Finish permission lifecycle behavior

Close and navigation-away cancellation are implemented. Remaining lifecycle
work covers grant expiry, audit logging, and explicit process-shutdown
sequencing.

Document which operations are denied after expiry, whether existing bodies are
aborted, what events are recorded, and what information must never enter logs
(paths, contents, or sensitive OS diagnostics). Test expiry, grant replacement,
controller destruction, process shutdown, pending setup, active readers, queued
data, and navigation authorizations.

### 7. Complete provider regression coverage

Retain regression coverage for worker-side read failures, full-channel
backpressure, four-reader concurrency, queue exhaustion, request aborts,
revocation races, and `GET` versus `HEAD` behavior under cancellation.

Use deterministic hooks or test doubles for races rather than relying only on
timing. Verify that every path releases file handles, permits, channels, and
blocking tasks.

### 8. Maintain integration guarantees

Keep security, browser, and `net` integration tests aligned with these
guarantees:

- local-file requests never use HTTP transport;
- cookies and service-worker interception are not used;
- file responses do not enter or read from the HTTP cache;
- unrelated scheme providers cannot satisfy a `file:` request;
- provider errors remain stable and non-diagnostic; and
- authorization occurs before path conversion for unauthorized requests.

Run the relevant cross-crate tests on supported platforms and include the new
size-limit, media-type, length-consistency, and session-lifecycle behavior in
the integration contract.

## Recommended order

1. Finish trusted-composition and path-authorization regression coverage.
2. Make revocation linearizable and finalize permission lifecycle behavior.
3. Define file-document origins and subresource policy.
4. Add duration/session-wide limits and active-content policy.
5. Define mutable-file semantics and complete provider/integration coverage.
