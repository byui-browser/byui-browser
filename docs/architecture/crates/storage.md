# Crate: `storage`

**Purpose:** client-side storage semantics (`localStorage` first, then
`sessionStorage`, IndexedDB, Cache API), scoped per origin and persisted to
disk behind a backend trait.
**Maintained by:** Security & Storage Team.
**Contract:** [`../contracts/networking.md`](../contracts/networking.md) §Storage.

## Dependencies

- May depend on: `common`, `security`.
- Depended on by: `webapis`, `browser`.
- Must never depend on: `net`, `webapis`, `js`.

## Intended public contract

See `contracts/networking.md` §`storage` public contract:
`LocalStorage::{open(StorageKey, Box<dyn StorageBackend>), get_item, set_item
(quota), remove_item, clear, len, handle(StorageRequest) -> StorageResponse}`,
`StorageBackend` trait, `InMemoryBackend`, `FileBackend`.

Key rules: every area is keyed by `security::StorageKey`; the
`common::ipc::Storage*` enums are the request/response shape so the API is
already process-boundary-shaped; persistence format is yours, location is
`browser`'s.

## Current state (2026-10-03)

About 110 lines, `NOT AUTHORITATIVE`.

- `LocalStorage { items: HashMap<String, String> }` with `new`, `get_item`,
  `set_item`, `remove_item`, `len`, `is_empty`. No `clear` (though
  `StorageRequest::Clear` exists), no origin scoping, no persistence, no use
  of `common::ipc` or `security::StorageKey` (G-17).
- A second, no-op `LocalStorage` exists in `webapis::local_storage` (G-13);
  only one should survive, and it is this one.
- Not used by any crate. `common` declared, unused (G-22).

## Gaps owned by this crate

G-17: add `security` dependency, key by `StorageKey`, add `clear`, implement
`handle(StorageRequest)`, put persistence behind `StorageBackend` with an
in-memory implementation first. The ignored contract test
`values_survive_reopen` is the acceptance test for persistence.

## Tests

- 4 unit tests (round trip, overwrite, remove, missing).
- `tests/contract.rs`: 1 active, 1 ignored (persistence).

## Read next

`contracts/networking.md` §Storage, `crates/security.md` (for `StorageKey`),
`crates/webapis.md` §Current state (your caller's duplicate type, G-13).
