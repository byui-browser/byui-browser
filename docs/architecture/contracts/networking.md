# Contract: Networking, Security policy, and Storage

**Teams:** Networking (`net`), Security & Storage (`security`, `storage`),
JS APIs (`webapis`, the main caller), Browser UX (`browser`, the other
caller).

## Division of labour

| Concern | Crate |
| --- | --- |
| Opening connections, TLS, HTTP versions, redirects, in-memory HTTP cache | `net` |
| Deciding *whether* a request may be made, which cookies attach, whether a response may be read (CORS) | `security` (pure functions and types; no I/O) |
| Holding cookies and site data between runs | `storage` (persistence) using `security` types |
| `Origin`, `StorageKey`, same-origin checks, cookie matching rules | `security` |
| `localStorage` / `sessionStorage` / IndexedDB semantics, per-origin, persisted | `storage` |
| Exposing all of the above to scripts | `webapis` |

The dependency direction is `net → security`, `storage → security`,
`webapis → {net, storage, security}`. `security` depends on nothing but
`common`, which is what lets every other crate call it without cycles.

## `security` public contract

```rust
pub struct Origin { /* scheme, host, port; opaque origins later */ }
impl Origin {
    pub fn parse(url: &str) -> Result<Origin, SecurityError>;   // total for any absolute http(s) URL
    pub fn is_same_origin(&self, other: &Origin) -> bool;
    pub fn is_potentially_trustworthy(&self) -> bool;             // https, localhost
}
pub struct StorageKey(Origin);                                     // partitioning key for storage

pub struct Cookie { pub name, pub value, pub domain, pub path, pub secure, pub http_only, pub same_site, pub expires }
pub struct CookieJar { /* ... */ }
impl CookieJar {
    pub fn store(&mut self, request_origin: &Origin, set_cookie: &str) -> Result<(), CookieError>; // parses Set-Cookie
    pub fn cookie_header_for(&self, url: &Url, is_secure: bool, same_site_context: SameSiteContext) -> Option<String>;
    pub fn export(&self) -> Vec<Cookie>; pub fn import(cookies: Vec<Cookie>) -> Self;   // for storage
}

pub fn cors_check(request_origin: &Origin, response_url: &Url, response_headers: &HeaderMap, mode: RequestMode) -> Result<(), CorsError>;
```

`security` never does I/O, never depends on `reqwest`, and is fully testable
with string inputs. That is what makes it the right home for policy.

## `net` public contract

```rust
pub struct Url(/* validated absolute URL */);
pub struct Request { pub method: Method, pub url: Url, pub headers: HeaderMap, pub body: Option<Vec<u8>>, pub cache_mode: CacheMode, pub context: FetchContext }
pub struct FetchContext { pub origin: security::Origin, pub mode: RequestMode, pub credentials: CredentialsMode }
pub struct Response { pub status: StatusCode, pub headers: HeaderMap, pub url: Url, pub body: Vec<u8>, pub from_cache: bool }

pub struct RequestController { /* shared: Clone */ }
impl RequestController {
    pub fn new(config: Config, cookies: Arc<Mutex<security::CookieJar>>) -> Result<Self, RequestError>;
    pub async fn fetch(&self, request: Request) -> Result<Response, RequestError>;
    pub fn clear_cache(&self);
}
```

Rules:

- `fetch` is `async` and runtime-agnostic. `net` never creates a runtime;
  `browser` owns one. Callers that must be synchronous (none should) are the
  caller's problem, not `net`'s.
- The request pipeline inside `fetch` is: validate URL/scheme → cache lookup
  → `security` attaches cookies → transport → `security::cors_check` →
  `security` stores `Set-Cookie` → cache store. Redirect hops go through the
  same policy steps; follow redirects in `net`, not in the HTTP library.
- `Method`, `HeaderMap`, `StatusCode` come from the `http` crate (which
  `reqwest` re-exports) and are an accepted part of the contract once ADR'd.
  `reqwest::Error` must not appear in a public signature; wrap it in
  `RequestError`.
- Policy stubs inside `net` (`CookieStore`, `CorsChecker`) are temporary and
  must be replaced by calls into `security`, not grown.
- Nothing above `net` opens a socket. `webapis` and `browser` only ever hold
  a `RequestController`.

## `storage` public contract

```rust
pub struct LocalStorage { /* per StorageKey */ }
impl LocalStorage {
    pub fn open(key: security::StorageKey, backend: Box<dyn StorageBackend>) -> Result<Self, StorageError>;
    pub fn get_item(&self, key: &str) -> Option<&str>;
    pub fn set_item(&mut self, key: &str, value: &str) -> Result<(), StorageError>;   // QuotaExceeded
    pub fn remove_item(&mut self, key: &str) -> Option<String>;
    pub fn clear(&mut self);
    pub fn len(&self) -> usize;
    pub fn handle(&mut self, request: common::ipc::StorageRequest) -> common::ipc::StorageResponse;
}
pub trait StorageBackend { fn load(&self, key: &StorageKey) -> Result<Vec<(String, String)>, StorageError>; fn persist(&self, key: &StorageKey, items: &[(String, String)]) -> Result<(), StorageError>; }
pub struct InMemoryBackend; pub struct FileBackend { root: PathBuf }
```

Rules:

- Every storage area is keyed by `security::StorageKey`. There is no
  unscoped storage.
- The `common::ipc::StorageRequest`/`StorageResponse` enums are the
  message shape `webapis` uses to talk to `storage`, so a later process split
  changes transport, not API.
- Persistence is behind a trait so tests use memory and the browser uses
  disk. The on-disk format is `storage`'s business; its location is
  `browser`'s.

## Current state (2026-10-03)

- `net` has its own no-op `CookieStore` and `CorsChecker`, does not depend
  on `security`, keys its cache by method + URL string, and exposes
  `reqwest::Error` from `RequestController::new`. `FetchContext.origin` is
  `Option<String>`. Crate-wide `#![allow(dead_code, unused_imports)]`.
  (G-14, G-15)
- `security::Origin::parse` and `CookieJar::{store, cookies_for}` are
  `todo!()`. `Cookie` lacks path/expiry/HttpOnly/SameSite. No CORS function.
  (G-16)
- `storage::LocalStorage` is an unscoped in-memory `HashMap` with no `clear`,
  no persistence, and no link to `common::ipc`. `webapis` has a second,
  no-op `LocalStorage`. (G-13, G-17)
