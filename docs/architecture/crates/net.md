# Crate: `net`

**Purpose:** fetch resources over HTTP(S): connections, TLS, redirects,
caching, concurrency limits. Asks `security` for policy; never decides policy
itself.
**Maintained by:** Networking Team.
**Contract:** [`../contracts/networking.md`](../contracts/networking.md).

## Dependencies

- May depend on: `common`, `security`; third-party `reqwest` (rustls, http2)
  and `tokio` as the current transport, pending any ADR to replace them.
- Depended on by: `webapis`, `browser`.
- Must never depend on: `webapis`, `storage`, `html`, anything above Layer 1.

## Intended public contract

See `contracts/networking.md` §`net` public contract: `Url`, `Request`,
`FetchContext { origin: security::Origin, .. }`, `Response`,
`RequestController::{new(config, cookie_jar), fetch (async), clear_cache}`,
`RequestError` (wrapping `reqwest::Error`, never exposing it), `Config`.

Key rules: async and runtime-agnostic; the pipeline inside `fetch` calls
`security` for cookies and CORS; redirects are followed by `net` so each hop
passes policy; cache keyed by URL + method + `Vary` headers with standard
freshness rules; only `http` crate types (`Method`, `HeaderMap`,
`StatusCode`) in public signatures.

## Current state (2026-10-03)

About 1,250 lines; the most complete non-engine crate, with a good
loopback-server test suite.

- `RequestController::new(Config) -> Result<Self, reqwest::Error>` (leaks
  `reqwest::Error`, G-15); `pub async fn fetch(&self, Request) -> Result<Response, RequestError>`;
  `clear_cache`.
- Pipeline: `RequestPolicy::validate_request` (URL parse, http/https only) →
  in-memory `ResponseCache` (key `"{method} {url}"`, `Cache-Control`
  `max-age`/`no-store`/`no-cache` only, success responses only) →
  `CookieStore::attach` (**no-op**) → `RequestScheduler` (tokio semaphore,
  priority accepted but ignored) → `ReqwestTransport` (buffers the whole body;
  reqwest follows redirects internally so hops bypass policy) →
  `CorsChecker::validate` (**no-op**) → `validate_response` (no-op) →
  `process_response` (no-op) → cache insert.
- `net` does **not** depend on `security`; cookie and CORS are local stubs
  whose comments defer to security/storage (G-14).
- `Request.url`/`Response.url` are `String`; `FetchContext.origin` is
  `Option<String>`; `RequestMode`/`CredentialsMode` are defined but never
  read (G-14).
- Public types expose `reqwest::Method`, `HeaderMap`, `StatusCode`,
  `HeaderValue` (acceptable as `http` types once ADR'd; G-15).
- Crate-wide `#![allow(dead_code)]` and `#![allow(unused_imports)]` (G-15).
- `common` declared, unused (G-22). Crate doc claims a Network process
  (G-28).

## Gaps owned by this crate

G-14 (add `security` dependency; replace `CookieStore`/`CorsChecker` with
calls into it; `Origin` in `FetchContext`; `Url` newtype), G-15 (wrap
`reqwest::Error`, remove crate-wide allows), G-30 (one test's
`Content-Length` mismatch and the hang-on-failure pattern), G-28.

## Tests

- 7 unit, 7 in-crate (`src/tests/`), 9 integration (`tests/request_controller.rs`).
  All use `TcpListener::bind("127.0.0.1:0")`; none need the internet. Good
  model for other teams.
- Nothing tests cookies, CORS, or scheduler priority (they are stubs).

## Read next

`contracts/networking.md`, `crates/security.md` (what you will call),
`crates/webapis.md` §Current state (your caller's blocking bug, G-10).
