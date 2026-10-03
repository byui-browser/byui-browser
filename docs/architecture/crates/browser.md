# Crate: `browser`

**Purpose:** the composition layer. The only crate that depends on
everything; it wires the pipeline, owns startup, the tokio runtime, and the
main loop, and exposes a `Browser` type that shells and tests drive. Ships a
thin CLI binary.
**Maintained by:** Browser UX Team, with Scrum of Scrums review on every PR
(it is where cross-team integration becomes real).
**Contract:** [`../contracts/ui.md`](../contracts/ui.md) §Composition layer.

## Dependencies

- May depend on: every workspace crate; `tokio` (owns the runtime).
- Depended on by: `platforms/*`, root `tests/`.
- Must stay a **library with a binary** (`src/lib.rs` + `src/main.rs`), not
  a binary alone.

## Intended public contract

See `contracts/ui.md`: `Browser::{new, open_tab, navigate, back, forward,
tick, render, chrome, console}`, `Engine` (one document's pipeline state),
`BrowserConfig`.

Key rules: all cross-crate wiring lives here; if two crates cannot depend on
each other, `browser` connects them through hooks. The binary loads a URL or
file, runs to idle, prints the console, optionally writes a PNG. Root
`tests/` end-to-end tests use the library.

## Current state (2026-10-03)

35 lines, binary only.

- Builds a `js::Realm`, registers `print` with a `StdoutConsole`
  (`println!`), builds a `net::RequestController`, registers `fetch`,
  evaluates `"print()"`, exits.
- Depends on all 13 crates in `Cargo.toml`; uses `js`, `webapis`, `net`.
  html/css/layout/paint/render/chrome/storage/security/devtools are declared
  and untouched (G-18).
- No library target, so `platforms/macos` cannot use it and the only
  pipeline test lives in `crates/layout/tests/` (G-18, G-27).

## Gaps owned by this crate

G-18: split into lib + bin; add `Engine` that runs
`parse → style → layout → paint → render` for a string of HTML (this makes
the layout pipeline test movable to root `tests/`); add `Browser` over
`chrome::TabManager`; own the tokio runtime and give `webapis` a
non-blocking path (with G-10). G-12: choose the console sink here.

## Tests

None. The first should be a root-level `tests/pipeline.rs` that loads
`tests/html/simple.html` through `Engine` and asserts on the `Frame`.

## Read next

`overview.md` §1–2, `contracts/ui.md`, then the crate doc of whatever you
are wiring in.
