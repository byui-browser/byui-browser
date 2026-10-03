# Crate: `devtools`

**Purpose:** developer-facing inspection: console store, DOM/style/layout
snapshots, network log. A consumer of every engine crate's read API; never a
dependency of any of them.
**Maintained by:** Devtools Team.
**Contract:** [`../contracts/ui.md`](../contracts/ui.md) §DevTools,
[`../contracts/scripting.md`](../contracts/scripting.md) §Console.

## Dependencies

- May depend on: `common`, `html`, `css`, `layout`, `js`, `webapis`
  (read-only inspection APIs).
- Depended on by: `browser`.
- Must never be depended on by an engine crate. If an engine crate needs to
  "tell DevTools something", it emits a `common` type and `browser` forwards it.

## Intended public contract

See `contracts/ui.md` §DevTools: `Console::{push(ConsoleMessage), entries,
filter(level)}`, `DomSnapshot::from(&html::Document)`,
`StyleSnapshot::for_node(&ComputedStyles, NodeId)`,
`LayoutSnapshot::from(&LayoutTree)`, `NetworkLog`.

Key rules: use `common::console::ConsoleMessage`, not a local level/entry
pair; snapshots are built from public read APIs, so if you need something
the engine crate does not expose, ask that team for a read accessor rather
than depending on internals; a wire protocol is a later layer.

## Current state (2026-10-03)

About 110 lines, `NOT AUTHORITATIVE`.

- `Level { Log, Warn, Error }`, `LogEntry { level, message }`,
  `Console { entries }` with `new`, `log`, `entries`, and `entries_at`
  (`todo!()` unless empty) (G-21).
- Nothing feeds it: `webapis::ConsoleSink` carries a bare `&str` and
  `browser` prints to stdout (G-12). No snapshots, no protocol.
- `common` declared, unused (G-22).

## Gaps owned by this crate

G-21 (implement `entries_at`/`filter`; un-ignore the contract test), G-12
jointly with Web APIs and Scrum of Scrums (define
`common::console::ConsoleMessage`, consume it here). Then the first snapshot
type once `html::Document` has a stable read API.

## Tests

- 2 unit tests (empty console; order and level preserved).
- `tests/contract.rs`: 1 ignored test (level filtering).

## Read next

`contracts/ui.md` §DevTools, `contracts/scripting.md` §Console,
`crates/webapis.md`.
