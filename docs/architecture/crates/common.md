# Crate: `common`

**Purpose:** the handful of types every crate needs and no single crate
produces: identifiers, the boundary error type, geometry primitives, console
and IPC message shapes.
**Maintained by:** all teams jointly; Scrum of Scrums arbitrates.
**Review:** every `pub` change needs the teams that use the item
(`.github/CODEOWNERS` requests all of them).

## Dependencies

- May depend on: nothing in the workspace, and no third-party crates.
- Depended on by: every crate.

## Intended public contract

| Module | Holds | Status |
| --- | --- | --- |
| `ids` | `NodeId(u32)` with `new`/`index`; later `TabId` if `chrome` and `browser` both need it | exists |
| `error` | `BrowserError { Parse{..}, Unimplemented(..), .. }`, `Result<T>` | exists, variants will grow |
| `geometry` | `Size`, `Rect`, `Point` (CSS px, `f32`), `Color` (RGBA8) | **to be created** (ADR 0003); today duplicated in `layout`, `paint`, `render` |
| `console` | `ConsoleLevel`, `ConsoleMessage` | **to be created**; see `contracts/scripting.md` §Console |
| `ipc` | `StorageRequest`, `StorageResponse`, `StorageError` | exists, unused |
| `dom` | **nothing**. The DOM lives in `html` (ADR 0002). This module should be deleted, not filled. | empty |

What does **not** go here: DOM nodes, style types, layout boxes, display
items, JS values, HTTP types, origins. Each is owned by its producing crate
(`overview.md` §3). The charter's "all cross-crate types live in `common`"
is the thing ADR 0003 narrows.

## Current state (2026-10-03)

- `ids::NodeId(u32)` exists and is used by `webapis`, `layout`, and parts of
  `html`, but `html`'s arena uses its own `html::NodeId(usize)` (G-01).
- `error::BrowserError` exists and is used by **no** other crate (G-22).
- `ipc::Storage*` exists and is used by **no** other crate (G-17).
- `dom.rs` is a single TODO comment (G-22).
- `common` is declared as a dependency by every crate and actually used by
  `html`, `css`, `layout`, `webapis` only (G-22).

## Tests

`tests/contract.rs` and `tests/ipc.rs` check that the public types are
usable as map keys, implement `Error`, and are `Clone + PartialEq`. Keep
them: they are the alarm that fires when a jointly owned type changes shape.

## Read next

`overview.md` §3 (shared types), then the crate doc of each consumer of the
item you are changing.
