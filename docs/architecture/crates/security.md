# Crate: `security`

**Purpose:** the policy crate: origins, same-origin checks, cookie rules,
CORS decisions, later CSP and permissions. Pure functions and types, no I/O,
so every other crate can call it.
**Maintained by:** Security & Storage Team.
**Contract:** [`../contracts/networking.md`](../contracts/networking.md).

## Dependencies

- May depend on: `common`.
- Depended on by: `net`, `storage`, `webapis`, `browser`.
- Must never depend on: `net`, `storage`, `reqwest`, or anything that does
  I/O. Being at Layer 0 is the whole point.

## Intended public contract

See `contracts/networking.md` §`security` public contract: `Origin::{parse,
is_same_origin, is_potentially_trustworthy}`, `StorageKey`, `Cookie` with
the full attribute set, `CookieJar::{store, cookie_header_for, export,
import}`, `cors_check`, `SecurityError`/`CookieError`.

Priority is the Canvas-login goal of the course: a session cookie must be
stored from `Set-Cookie`, matched by domain/path/secure/expiry, and sent
back. That is `Origin::parse` + `CookieJar` first; CORS second; CSP and
permissions later.

## Current state (2026-10-03)

About 290 lines, single `lib.rs`, mostly `NOT AUTHORITATIVE`.

- `Origin { scheme, host, port }` with `new` (lowercases, default ports 80/443,
  **0 for other schemes**), `with_port`, getters, `placeholder()`.
  `Origin::parse(url)` rejects `""` and otherwise **`todo!()`** (G-16).
- `same_origin(a, b)` is `a == b`. `StorageKey(Origin)` exists.
- `Cookie { name, value, domain, secure }` lacks path, expiry, HttpOnly,
  SameSite. `CookieJar::store` is `todo!()`; `cookies_for` is `todo!()`
  unless empty (G-16).
- No CORS, CSP, permissions, or sandboxing code; the crate doc claims
  several of these (G-28).
- Not used by any crate (`browser` declares it, `net` does not). `common`
  declared, unused (G-22).

## Gaps owned by this crate

G-16 is on the critical path for the course goal and for `net`'s G-14:
implement `Origin::parse` for absolute http(s) URLs, fill out `Cookie`,
implement `CookieJar::store` from a `Set-Cookie` string and
`cookie_header_for`. Then `cors_check`. Coordinate with Networking on the
exact function signatures before either side builds.

## Tests

- 8 unit tests (origin normalisation, same-origin, storage key, empty jar).
- `tests/contract.rs`: 3 tests, all ignored (origin parsing, path ignored
  for origin, secure cookie not sent over http). These are the acceptance
  tests for G-16; un-ignore them as you go.

## Read next

`contracts/networking.md`, `crates/net.md` §Current state (the stubs you
replace), `crates/storage.md`.
