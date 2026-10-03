# Crate: `webapis`

**Purpose:** every JavaScript-visible browser API (`document`, `fetch`,
timers, `localStorage`, `console`) and the event loop scripts observe. The
bridge between `js` and the rest of the engine.
**Maintained by:** JS APIs (Web APIs) Team.
**Contracts:** [`../contracts/scripting.md`](../contracts/scripting.md)
(primary); it also calls into [`dom.md`](../contracts/dom.md) and
[`networking.md`](../contracts/networking.md).

## Dependencies

- May depend on: `common`, `security`, `html`, `js`, `net`, `storage`.
- Depended on by: `browser`, `devtools`.
- Must never depend on: `chrome`, `devtools`, `browser`, `tokio` directly
  (asynchrony is delivered through `net`'s futures and `browser`'s runtime).

## Intended public contract

See `contracts/scripting.md` §`webapis` public contract:
`HostServices`, `ConsoleSink`, `ScriptHost::{new, run_script, run_pending_tasks, take_dom_mutations}`.

Key rules: one registration entry point; nothing blocks; `document` binds
the live `html::Document` through its mutation API; dotted names are object
properties; console output is a `common::console::ConsoleMessage`; timers
and fetch completions are tasks on the `webapis` event loop.

## Current state (2026-10-03)

About 430 lines.

- `ConsoleSink { fn log(&self, &str) }` and `print(console, args)` /
  `register_print(realm, Arc<dyn ConsoleSink>)`: zero-argument, logs the
  fixed string `JS API Team Rocks!`. The injected sink is the right idea;
  the message carries no level (G-12). The global name `print` is a
  placeholder (G-29).
- `fetch(controller, args)` / `register_fetch`: takes one URL string,
  **creates a new tokio multi-thread runtime per call and `block_on`s**
  `net::RequestController::fetch`, returns the body as a `String`. Will
  panic if called from inside a runtime. (G-10)
- `Document { elements: Vec<Element> }` built by `from_html_document` is a
  flattened **snapshot** of elements with `id` attributes; `get_element_by_id`
  works on the snapshot. No `document` global is registered; no mutation.
  Converts `html::NodeId` to `common::ids::NodeId` with `as u32` (G-01, G-11).
- `TimerQueue::{schedule, pop_next_due}` are `todo!()` behind an ignored
  contract test.
- `local_storage.rs`: a second `LocalStorage` unit struct (no-op), and
  `register_local_storage` installs flat globals literally named
  `"localStorage.setItem"` / `"localStorage.getItem"`. No doc comments.
  (G-11, G-13)
- Depends on `tokio` directly.

## Gaps owned by this crate

G-10 (unblock first: it is a correctness bug waiting for the first page that
runs two fetches), G-11 (single `ScriptHost::new`, bind the live DOM, event
loop), G-12 (level-carrying console message, with DevTools), G-13 (delete
the local `LocalStorage`, bind `storage::LocalStorage`), G-29 (rename
`print` → `console.log` when objects exist). Blocked on `js` G-09 for
anything that needs arguments or callbacks from script.

## Tests

- 8 unit tests in `lib.rs`, 5 in `local_storage.rs`, `tests/print_binding.rs`
  (1), `tests/contract.rs` (1, ignored: timers).
- No test performs a successful fetch; `net`'s loopback-server pattern is
  the way to add one.

## Read next

`contracts/scripting.md`, `crates/js.md` §Current state (what you can rely
on), `contracts/dom.md` §Rules for consumers, `contracts/networking.md`.
