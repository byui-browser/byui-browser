# Crate: `chrome`

**Purpose:** the browser's UI **state**: tabs, per-tab navigation history,
address bar, later bookmarks and settings. Pure data and logic; no windowing.
**Maintained by:** Browser UX Team.
**Contract:** [`../contracts/ui.md`](../contracts/ui.md).

## Dependencies

- May depend on: `common`.
- Depended on by: `browser`.
- Must never depend on: any engine crate, any windowing or drawing library.
  Shells draw `chrome` state; `chrome` does not draw.

## Intended public contract

See `contracts/ui.md` §Browser UI state: `TabId`, `Tab { id, title, history,
loading }`, `NavigationHistory::{current, can_go_back, can_go_forward, push,
back, forward}`, `TabManager::{open, close -> Option<TabId>, activate,
active, tabs}`, `AddressBar`.

Key rules: `TabManager` allocates ids; closing the active tab activates a
neighbour; history is per tab; everything derives `Debug, Clone, PartialEq`
so shells and tests can compare states.

## Current state (2026-10-03)

About 120 lines, `NOT AUTHORITATIVE`.

- `TabId(pub u32)`, `Tab { id, url: String }`, `TabManager { tabs, active }`
  with `new`, `active`, `tab_count`. `open` is `todo!()`; `close` is
  `todo!()` unless empty. No id allocator, no history, no title, no address
  bar (G-19).
- `DEFAULT_TAB_TITLE` is unused. Nothing in the workspace uses `chrome`;
  `platforms/macos` draws Back/Forward buttons without consulting it (G-20).
- `common` declared, unused (G-22).

## Gaps owned by this crate

G-19: implement `TabManager` (un-ignore the three contract tests), add
`NavigationHistory`, then `AddressBar`. Coordinate with the shell work so
Back/Forward enablement comes from here (G-20).

## Tests

- 2 unit tests (empty window, close on empty is a no-op).
- `tests/contract.rs`: 3 ignored tests (open activates, close activates
  neighbour, close last leaves none). These are the spec for `TabManager`.

## Read next

`contracts/ui.md`, `crates/browser.md`, `crates/platforms.md`.
