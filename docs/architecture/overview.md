# Target architecture

This document describes the browser we are building toward: how the crates
layer, what flows between them, and the rules that keep nine teams' work
composable. It is deliberately the *ideal*. Where the code differs today, the
per-crate documents under [`crates/`](crates/) say so, and
[`gap-report-2026-10.md`](gap-report-2026-10.md) lists every known difference.

The charter in `docs/TECH_ARCHITECTURE.md` set the principles (memory safety
first, clear ownership, message passing, correctness before performance, a
platform-agnostic core). This document turns those principles into concrete
rules. Where it proposes changing something the charter says, it says so and
points at the ADR that must land first.

## 1. The pipeline

A page load is one pass through this chain. Every arrow is a function call
that passes plain data owned by the crate on the left.

```
  URL ──► net ──► bytes ──► html ──► Document (DOM arena)
                                        │
             css::parse_stylesheet ◄────┤ <style>, <link>, style=""
                      │                 │
                      ▼                 ▼
             css::style_document(&Document, &[Stylesheet]) ──► ComputedStyles
                                        │
                                        ▼
             layout::layout_tree(&Document, &ComputedStyles, viewport) ──► LayoutTree
                                        │
                                        ▼
             paint::build_display_list(&LayoutTree) ──► DisplayList
                                        │
                                        ▼
             render::Compositor::compose(&DisplayList, scale) ──► Frame (RGBA8)
                                        │
                                        ▼
                     chrome / platform shell presents the Frame
```

Scripts run beside the pipeline, not inside it:

```
  <script> source ──► js::parse ──► js::Realm::evaluate (interpreter)
                                        │ host function calls
                                        ▼
                      webapis (document, fetch, timers, localStorage, console)
                        │          │          │            │          │
                        ▼          ▼          ▼            ▼          ▼
                     html DOM     net     event loop    storage    devtools
                     mutation                 (webapis)            console
                        │
                        ▼
              invalidation ──► re-run css → layout → paint → render
```

Three consequences of drawing it this way:

- **Each stage is a pure-ish function of its inputs.** `css` does not reach
  into `layout`; `paint` does not re-read the DOM. If a stage needs
  information, the stage before it puts that information in its output type.
- **Mutation happens in exactly one place**: the DOM, through `html`'s
  mutation API. Everything downstream is recomputed (today wholesale, later
  incrementally via dirty flags).
- **The event loop belongs to `webapis`.** Timers, fetch completions and
  (later) DOM events are tasks on that loop; `js` only knows how to run a
  function when asked.

## 2. Layering and dependency rules

Crates are arranged in layers. A crate may depend only on crates in the same
row or below. This is the single most important rule in the repository, and
it is checked by reading `Cargo.toml` in review.

```
 Layer 5  platforms/*  (thin native shells)
 Layer 4  browser      (composition: wires everything, owns startup)
 Layer 3  chrome   devtools
 Layer 2  webapis
 Layer 1  html  css  layout  paint  render  js  net  storage
 Layer 0  common  security
```

Allowed edges (anything not listed is forbidden until an ADR adds it):

| Crate | May depend on | Is depended on by |
| --- | --- | --- |
| `common` | nothing in the workspace | everything |
| `security` | `common` | `net`, `storage`, `webapis`, `browser` |
| `html` | `common` | `css`, `layout`, `webapis`, `devtools`, `browser` |
| `css` | `common`, `html` | `layout`, `devtools`, `browser` |
| `layout` | `common`, `html`, `css` | `paint`, `devtools`, `browser` |
| `paint` | `common`, `layout` | `render`, `browser` |
| `render` | `common`, `paint` | `browser`, `platforms/*` (for `Frame`) |
| `js` | `common` | `webapis`, `devtools`, `browser` |
| `net` | `common`, `security` | `webapis`, `browser` |
| `storage` | `common`, `security` | `webapis`, `browser` |
| `webapis` | `common`, `security`, `html`, `js`, `net`, `storage` | `browser`, `devtools` |
| `chrome` | `common` | `browser` |
| `devtools` | `common`, `html`, `css`, `layout`, `js`, `webapis` (read-only inspection APIs) | `browser` |
| `browser` (lib + bin) | every crate above | `platforms/*` |
| `platforms/*` | `browser`, `render` | nothing |

Explicitly forbidden, because each has been tempting at some point:

- `js → webapis` (the engine must not know about any particular Web API).
- `paint → html` or `render → html` (layout's output must carry text runs and
  colors; paint never consults the DOM).
- `layout → html` without `css` (layout consumes computed style, not raw
  markup). Today layout reads `html` directly as a stepping stone; see the gap
  report.
- `net → webapis`, `net → storage` (net is a transport; it asks `security`
  for policy and returns bytes).
- Any Layer 0–2 crate depending on `chrome`, `devtools`, or `browser`.
- Any engine crate depending on a windowing or GPU library. Only `render`
  (GPU, later) and `platforms/*` (windowing) may.

Dev-dependencies are exempt from the direction rule when they only exist to
run an end-to-end test (for example `layout` pulling `paint` and `render` in
`[dev-dependencies]`), but the end-to-end test itself belongs in `tests/` at
the repository root once a `browser` library exists to host it.

## 3. Shared types: who owns what

The charter's §3.1 says "all cross-crate types live in `common`". We propose
narrowing that, and the change needs an ADR (see §7). The narrower rule:

> A type is owned by the crate that **produces** it. `common` holds only
> identifiers, errors, geometry primitives, and message enums that no single
> engine crate produces.

Reasoning: a crate that owns every shared type is a crate every team must
review on every change, which is exactly the bottleneck a nine-team project
cannot afford. Rust lets a consumer depend on the producer's type directly,
with no cost, as long as the dependency direction in §2 is respected.

So the intended homes are:

| Type | Home | Consumers |
| --- | --- | --- |
| `NodeId` | `common::ids` | every crate that names a DOM node |
| `BrowserError`, `Result` | `common::error` | every crate boundary |
| `Size`, `Rect`, `Point`, `Color` (CSS px, RGBA8) | `common::geometry` (to be created) | `css`, `layout`, `paint`, `render`, `chrome` |
| `ConsoleMessage`, `ConsoleLevel` | `common::console` (to be created) | `webapis` (produces), `devtools`, `browser` (consume) |
| `StorageRequest`, `StorageResponse`, `StorageError` | `common::ipc` | `storage`, `webapis` |
| `Document`, `Node`, `NodeKind`, `ElementData`, `Attribute` | `html` | `css`, `layout`, `webapis`, `devtools` |
| `Stylesheet`, `Rule`, `Declaration`, `ComputedStyle`, `ComputedStyles` | `css` | `layout`, `devtools` |
| `LayoutTree`, `LayoutBox`, `TextRun` | `layout` | `paint`, `devtools` |
| `DisplayList`, `DisplayItem` | `paint` | `render` |
| `Frame` | `render` | `browser`, `platforms/*` |
| `Value`, `Realm`, `HostFunction`, `JsError` | `js` | `webapis`, `devtools` |
| `Request`, `Response`, `RequestError`, `Url` | `net` | `webapis`, `browser` |
| `Origin`, `StorageKey`, `Cookie`, `CookieJar`, policy fns | `security` | `net`, `storage`, `webapis` |
| `Tab`, `TabId`, `TabManager`, `NavigationHistory` | `chrome` | `browser` |

Rules for anything in `common`:

- Changing a `pub` item in `common` requires review from every team that uses
  it (`.github/CODEOWNERS` already enforces the request; reviewers must
  actually check the callers).
- `common` has no dependencies and no behaviour beyond trivial constructors,
  `Display`, and conversions.
- Nothing goes into `common` "because two crates might want it later". It goes
  in when the second consumer exists.

## 4. Process model: target and present

The charter calls for a multi-process browser from day one. The code is a
single process, and that is the right place to be at this stage of the course.
We propose formalising that (ADR 0001, proposed):

- **Now and for the foreseeable semester:** one process, one thread for the
  engine pipeline, a tokio runtime owned by `browser` for networking, and a
  task queue owned by `webapis` for script-visible asynchrony.
- **Shape every boundary as if it were an IPC boundary anyway.** The types
  that cross between `webapis` and `storage`, or between the renderer side
  and `net`, must be plain data (`Clone`, no references into another crate's
  state, no closures). `common::ipc` already models storage this way; new
  boundaries follow it. This is what makes a later process split a refactor
  rather than a rewrite.
- **Crash isolation comes from the type system first:** no `unwrap()` on
  external input, no `todo!()` reachable from a loaded page, errors surfaced
  as `BrowserError` and shown in the chrome.

Nothing in this document depends on the process split happening this year.

## 5. Threading and asynchrony

- The engine pipeline (`html → css → layout → paint → render`) is synchronous
  and single-threaded. Parallel style or layout is a later optimisation and
  must not leak into the public signatures.
- `net` is `async` and runtime-agnostic in its API; it requires a tokio
  runtime to be present but never creates one. `browser` owns that runtime.
- `webapis` must never block the engine thread on a network call. `fetch`
  returns a pending value (a `Promise` once `js` has objects; until then an
  explicit task handle) and the completion is delivered through the
  `webapis` event loop.
- `js` is single-threaded and `!Send` is acceptable for interpreter state.
  Host functions registered into a `Realm` therefore need not be `Send`;
  requiring `Send + Sync` today is a constraint we expect to drop.

## 6. Error handling

- Inside a crate, use whatever error type fits. At a `pub` boundary that
  another team calls, return `common::error::BrowserError` or a crate error
  that implements `std::error::Error` and converts `Into<BrowserError>`.
- Parsers (`html`, `css`, `js`) are **total**: they never fail on malformed
  input. They recover the way real browsers do and record diagnostics.
  Returning an error from a parser is a bug, except for `js`, where a syntax
  error is a specified outcome.
- `todo!()` and `unimplemented!()` are placeholders for the scaffold phase.
  See [`conventions.md`](conventions.md) for when they are allowed.

## 7. Decisions that need an ADR

Record these in `docs/adr/` before or alongside the code. Two are drafted as
*Proposed* with this document:

| # | Decision | Status |
| --- | --- | --- |
| 0001 | Single-process browser with IPC-shaped boundaries (supersedes charter §1.2 "from day one") | Proposed, drafted |
| 0002 | DOM representation: one arena in `html`, addressed by `common::ids::NodeId` | Proposed, drafted |
| 0003 | Shared-type ownership: producer-owned types, `common` limited to ids/errors/geometry/messages (narrows charter §3.1) | Needed |
| 0004 | Script asynchrony: `webapis` owns the event loop; `fetch` never blocks | Needed |
| 0005 | UI toolkit for `chrome` and the shells (winit + softbuffer vs. native) | Needed before chrome grows |
| 0006 | GPU backend for `render` (software now; wgpu when?) | Needed before any GPU code |
| 0007 | Cookie jar ownership between `security` (policy) and `storage` (persistence) | Needed |

Anything that adds an edge to the table in §2, moves a type between crates,
or changes a signature in a `contracts/*.md` file needs at least a Scrum of
Scrums decision, and usually an ADR.

## 8. Reading order for a new contributor

1. This document, once.
2. `docs/TEAMS.md` to find your team.
3. Your crate's document under [`crates/`](crates/).
4. The contract documents your crate appears in.
5. [`conventions.md`](conventions.md) before your first PR.
6. The gap report rows tagged with your team, to pick up work.
