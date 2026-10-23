# Browser `file:` provider

The browser adapts security-owned file access to `net::FileProvider`. The
security crate owns path authorization and filesystem capabilities; the browser
owns request-context translation, grants, and session lifecycle; `net` owns
scheme dispatch, response validation, and body behavior.

## Current status

The provider is a default-deny, capability-backed implementation for trusted
roots. It requires browser-issued single-use navigation authorization, opens
files through retained handles with symlink following disabled, streams bounded
64 KiB chunks, limits admission to four readers and eight pending opens, and
supports request abort plus session revocation. Core authorization, containment,
streaming, and lifecycle tests pass. The remaining work below is required
before treating the provider as a complete browser capability.

Browser configuration consumes opaque `TrustedDirectorySelection` values with
explicit browser-owned resource limits. The selection's path and directory
handle remain private to the security capability after configuration, and the
browser never reopens the selected pathname. Platform-specific user permission
selection remains owned by the platform team.

## Remaining work

### 1. Remove path-existence oracles

Path authorization currently canonicalizes a requested path before checking
whether it belongs to an allowed root. An authorized caller can therefore
distinguish an existing outside-root path (`AccessDenied`) from a missing
outside-root path (`ResourceMissing`).

Establish lexical/root eligibility before filesystem canonicalization where
possible. For paths that are outside the configured roots, normalize missing,
permission, and related lookup failures to `AccessDenied`. Preserve useful
`ResourceMissing` behavior for paths that are eligible under an allowed root.

Test existing and missing outside-root paths, malformed paths, traversal, and
symlinked paths. Verify that unauthorized and authorized callers receive the
same non-diagnostic denial for ineligible paths.

### 2. Make revocation fully linearizable

Session revocation now cancels pending setup and live bodies, but authorization,
file opening, and response construction still need one clear revocation
linearization point. A revocation racing with the final setup check must not
publish a `HEAD` response or a queued body chunk from the old session.

Carry a session generation or revocation token into the security-owned open
operation. Revalidate it before admitting the opened handle and before exposing
response metadata or body data. Define whether an already-running OS read may
finish internally, but ensure its bytes are never delivered after the session
is revoked.

Add race-focused tests for revocation before opening, during blocking setup,
immediately before response construction, while a chunk is queued, and during
`HEAD` handling. Cover close, navigation-away, grant replacement, and grant
expiry consistently.

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

### 4. Add resource and content limits

Define product limits for maximum file size, total bytes served per session,
read duration, active readers, pending requests, chunk size, and aggregate
blocking-reader/channel memory. Enforce limits before opening when possible and
while streaming when limits depend on bytes or time.

Add filesystem media-type detection and specify whether invalid detected values
are rejected or omitted. Define MIME sniffing, `nosniff`, and executable-content
rules before local HTML, JavaScript, images, stylesheets, or other active
content is interpreted. Decide whether directories and unsupported file types
retain the current non-diagnostic `Io` category or receive dedicated errors.

Test each limit at its boundary, including concurrent readers, queue
exhaustion, slow reads, and cancellation while a limit is being reached.

### 5. Define stream consistency

The provider captures `Content-Length` when the handle is opened but does not
currently enforce that length while reading. Truncation or extension can make
the streamed body differ from the advertised length.

Choose whether local-file responses require a stable snapshot or support live
file contents. If the captured length is authoritative, stop after that many
bytes and report a consistency failure when the file changes. If live contents
are supported, define the response framing and security implications explicitly.

Also define behavior for in-place writes, truncation, growth, and metadata
changes after authorization. Add tests for each mutation during setup and
streaming, including cleanup of the retained handle and read permit.

### 6. Finish permission lifecycle behavior

Specify grant expiry, audit logging, and process-shutdown behavior. Document
which operations are denied after expiry, whether existing bodies are aborted,
what events are recorded, and what information must never enter logs (paths,
contents, or sensitive OS diagnostics).

Define explicit shutdown sequencing if process teardown is not sufficient for
future browser lifecycle code. Test tab close, navigation-away, grant expiry,
grant replacement, controller destruction, and process shutdown for pending
setup, active readers, queued data, and navigation authorizations.

### 7. Complete streaming regression coverage

Add browser-provider tests for:

- worker-side read failures and stable error mapping;
- full-channel backpressure and cancellation while blocked;
- all four active readers and eight pending opens;
- queue exhaustion and permit release after failure;
- unread and partially read body drops;
- request abort before admission, during setup, and during streaming;
- session revocation at every lifecycle boundary; and
- `GET` versus `HEAD` behavior under cancellation and revocation.

Use deterministic hooks or test doubles for races rather than relying only on
timing. Verify that every path releases file handles, permits, channels, and
blocking tasks.

### 8. Maintain integration and conformance guarantees

Keep security, browser, and `net` integration tests aligned with these
guarantees:

- local-file requests never use HTTP transport;
- cookies and service-worker interception are not used;
- file responses do not enter or read from the HTTP cache;
- unrelated scheme providers cannot satisfy a `file:` request;
- provider errors remain stable and non-diagnostic; and
- authorization occurs before path conversion for unauthorized requests.

Run the relevant security, browser, and `net` tests on supported platforms and
include platform-specific coverage for Unix mounts and links, Windows
reparse-point variants, drive-letter and UNC paths, network shares, and
removable media.

## Recommended order

1. Remove the path-existence oracle.
2. Make revocation linearizable and finalize permission lifecycle behavior.
3. Define file-document origins and subresource policy.
4. Add resource limits, content handling, and stream consistency.
5. Complete race-focused tests and cross-crate/platform conformance coverage.
