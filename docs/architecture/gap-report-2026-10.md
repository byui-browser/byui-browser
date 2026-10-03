# Gap report: repository vs. target architecture (October 2026)

**Snapshot of:** `main` at commit `8971cb2` (after PR #134), 2026-10-03.
**Method:** every `.rs` file in `crates/` and `platforms/` was read against
[`overview.md`](overview.md) and the `contracts/` documents.
**Audience:** Scrum of Scrums (tracking), team leads (sprint planning),
agents (so they do not rebuild on a gap by accident).

The project is early, and most of these gaps are the expected distance
between scaffold and engine. The ones marked **High** are the ones where
building further on the current shape would make the later fix more
expensive; they should be scheduled before feature work in that area.

**Severity:** High = blocks a contract or will force rework if built upon;
Medium = wrong shape but contained; Low = cleanup or documentation.
**Status:** Open until a PR closes it; mark `Closed (#PR)` in place.

## Cross-cutting

| ID | Gap | Severity | Owner | Resolution | Status |
| --- | --- | --- | --- | --- | --- |
| G-01 | Two `NodeId` types: `html::NodeId(pub usize)` for the arena vs `common::ids::NodeId(u32)` everywhere else. `webapis`, `layout` (`html_node_id`) and html's own `HTMLElement` cast between them with `as`. | High | HTML (lead), Layout, Web APIs | `html` adopts `common::ids::NodeId`; delete `html::NodeId` and `layout::html_node_id`; fix call sites in one PR. ADR 0002. | Open |
| G-07 | Duplicate types for one concept: `layout::Rect` / `paint::Rect`; `paint::DisplayItem` (f32) / `render::DisplayItem` (u32); `html::Dom` / `css::Dom`; `storage::LocalStorage` / `webapis::LocalStorage`. | Medium | Layout & Rendering; CSS; Web APIs | One owner per type per `overview.md` §3; delete the rest. Geometry moves to `common::geometry` under ADR 0003. | Open |
| G-12 | No shared console message. `webapis::ConsoleSink::log(&str)` has no level; `devtools` has its own `Level`/`LogEntry`; `browser` prints to stdout. | Medium | Web APIs, DevTools, Scrum of Scrums (for `common`) | Add `common::console::{ConsoleLevel, ConsoleMessage}`; `webapis` emits it, `devtools::Console` stores it, `browser` chooses the sink. | Open |
| G-22 | `common` is declared by every crate but used by four; `common::dom` is an empty TODO; `common::error::BrowserError` is used by no other crate. | Low | All | Remove unused `common` deps; delete `common::dom` (ADR 0002 puts the DOM in `html`); route boundary errors through `BrowserError` as APIs firm up. | Open |
| G-23 | Charter §1.2 says multi-process "from day one"; the code is one process with no IPC and that is appropriate for now. The docs and code disagree. | Medium (docs) | Scrum of Scrums | Accept ADR 0001 (single process, IPC-shaped boundaries). Update charter §1.2. | Open, ADR drafted |
| G-24 | Charter §3.1 says all cross-crate types live in `common`; in practice none do, and the rule would make `common` a review bottleneck. | Medium (docs) | Scrum of Scrums | Write and accept ADR 0003 (producer-owned types). Update charter §3.1. | Open |
| G-25 | Stale `#[ignore]` reasons on contract tests that now pass (verified with `cargo test -p <crate> --test contract -- --ignored`): `css` (2), `layout` (1), `render` (1). Stale doc comments on `css::parse_stylesheet` and `css::specificity` ("only handles the empty ..."). | Low | CSS, Layout & Rendering | Un-ignore, fix docs. Quick wins. | Open |
| G-26 | Dead or demo code: `crates/layout/src/basics.rs` (undeclared `fn main` with `println!`), `crates/html/src/simple_html_parser.rs` (undeclared third parser, does not compile), `crates/html/example.html` (unused), `crates/js/src/main.rs` (demo binary, nine `println!`s), `eprintln!` in `layout/tests/simple_element_pipeline.rs`. | Low | Layout & Rendering, HTML, JS Engine | Delete. Demo code becomes an `examples/` entry or a test. | Open |
| G-27 | The only HTML-to-pixels test lives in `crates/layout/tests/` and pulls `paint`/`render` as dev-deps; root `tests/` holds fixtures only. | Low | Layout & Rendering, Browser UX | Move to root `tests/pipeline.rs` once `browser` has a library target (G-18). | Open, blocked on G-18 |
| G-28 | Crate docs describe things that do not exist: `js` ("bytecode compiler, interpreter/VM, GC"), `render` ("GPU rendering (wgpu)"), `security` ("sandboxing, process isolation, CSP, permissions"), `net` ("all network goes through the Network process"). | Low | JS Engine, Layout & Rendering, Security & Storage, Networking | Describe what exists and link the crate doc for the target. | Open |

## HTML

| ID | Gap | Severity | Owner | Resolution | Status |
| --- | --- | --- | --- | --- | --- |
| G-02 | Three DOM models in `html`: arena `HtmlDocument`; legacy flat `Dom`/`LegacyNode`/`HTMLElement` with `tokenize`/`parse`; dead `simple_html_parser.rs`. Three copies of the void-element list. | High | HTML | Keep the arena. Delete the legacy model and `tokenize`/`parse` (update `html/tests/contract.rs`), delete the dead parser. | Open |
| G-03 | No mutation API beyond `push_node`/`append_child` (panics on bad id, does not unlink); no mutation log; `QuirksMode` never computed; no implied `<html>/<head>/<body>`, character references, or raw-text elements. | High | HTML | Mutation API + `take_mutations` per `contracts/dom.md`; then parser conformance. | Open |
| G-32 | `Query` trait with `#[allow(non_snake_case)] fn Query` that scans detached nodes; selector-matcher ownership between HTML and CSS undecided. | Medium | HTML, CSS | Remove the `Query` trait (keep `query_selector`). Decide matcher home per `contracts/dom.md` §Selectors before either team adds a second matcher. | Open |

## CSS

| ID | Gap | Severity | Owner | Resolution | Status |
| --- | --- | --- | --- | --- | --- |
| G-04 | `css::style_document(&css::Dom, ...)` takes a placeholder `Vec<NodeId>` with no structure, hits `todo!()` on any input, and nothing consumes `ComputedStyles`. `css` does not depend on `html`. `Declaration` is `String`/`String`. | High | CSS | Depend on `html`; take `&HtmlDocument`; implement matching for type/class/id; typed `Property`/`Value` for the first properties; agree `ComputedStyle` fields with Layout. | Open |

## Layout & Rendering

| ID | Gap | Severity | Owner | Resolution | Status |
| --- | --- | --- | --- | --- | --- |
| G-05 | `layout` depends on `html`, not `css`; `StyledDom` is a `Vec<NodeId>` with no style; every element is a 100 px full-width block; nesting ignored. | High | Layout & Rendering | `layout_tree(&Document, &ComputedStyles, Size)`; real block layout. Blocked on G-04 for styles; structure work can start now. | Open |
| G-06 | `paint` depends on `html` and reads DOM text via `html_node_id`; colours are constants. | Medium | Layout & Rendering | `LayoutBox` carries `TextRun` and `background`; `paint` drops `html`. | Open |
| G-08 | `render` has no scale factor; converts CSS px to device px with `as u32`; two `DisplayItem` types (see G-07). | Medium | Layout & Rendering | `Compositor::new(w, h, scale)`; consume `paint::DisplayList` directly. | Open |

## JavaScript Engine

| ID | Gap | Severity | Owner | Resolution | Status |
| --- | --- | --- | --- | --- | --- |
| G-09 | `js::Realm` is a `HashMap` of host functions disconnected from the interpreter. `evaluate_script` string-matches `name()` and cannot pass arguments. The interpreter rejects every call, `if`, `while`, `function`, `return`; all operators coerce to number (`+` never concatenates; `'a' == 'a'` errors). | High | JS Engine | `Realm` owns the interpreter environment; `evaluate_script` parses and runs; `Expr::Call` resolves globals and invokes host functions through the normal call path. Then control flow and string semantics. Everything in `webapis` waits on this. | Open |
| G-31 | `HostFunction = Arc<dyn Fn(&[Value]) + Send + Sync>` with no `HostContext`; forces `webapis` to use `Arc`/`Mutex` for state the single-threaded interpreter never shares. | Medium | JS Engine, Web APIs | `Box<dyn FnMut(&mut HostContext, &[Value])>`, not `Send`. Do with G-09. | Open |

## Web APIs

| ID | Gap | Severity | Owner | Resolution | Status |
| --- | --- | --- | --- | --- | --- |
| G-10 | `webapis::fetch` creates a new tokio multi-thread runtime **per call** and `block_on`s `net`. Blocks the engine; panics if called inside a runtime. | High | Web APIs, Browser UX (runtime owner) | `fetch` enqueues on `net`'s future and completes on the `webapis` event loop; `browser` owns the runtime. ADR 0004. | Open |
| G-11 | Registration split across `register_print`/`register_fetch`/`register_local_storage`; `"localStorage.setItem"` is a flat global with a dot in its name; `webapis::Document` is a flattened snapshot of elements with no live link or mutation path; no `document` global. | High | Web APIs | `ScriptHost::new(HostServices)` as the one entry point; bind the live `html::Document`; objects wait on `js` `Value::Object`. | Open |
| G-13 | `webapis::local_storage::LocalStorage` duplicates `storage::LocalStorage` as a no-op with no doc comments. | Medium | Web APIs, Security & Storage | Delete the `webapis` copy; bind `storage::LocalStorage` keyed by the page origin. | Open |
| G-29 | The global is named `print` and logs a fixed string. `window.print` means "print dialog" on the web. | Low | Web APIs | Rename to `console.log` when objects exist; until then keep as documented placeholder. | Open |

## Networking

| ID | Gap | Severity | Owner | Resolution | Status |
| --- | --- | --- | --- | --- | --- |
| G-14 | `net` does not depend on `security`; cookies and CORS are no-op stubs inside `net`; `FetchContext.origin` is `Option<String>`; `RequestMode`/`CredentialsMode` are never read; cache key is `"{method} {url}"` with `max-age` only; reqwest follows redirects so hops bypass policy. | High | Networking, Security & Storage | Add `security` dep; call `CookieJar`/`cors_check`; `Origin` in `FetchContext`; `Url` newtype; follow redirects in `net`. Needs G-16 landed or co-developed. | Open |
| G-15 | `RequestController::new` returns `reqwest::Error`; crate-wide `#![allow(dead_code)]` and `#![allow(unused_imports)]`. | Medium | Networking | Wrap in `RequestError`; remove the allows and fix what they hide. | Open |
| G-30 | Test `no_store_requests_are_not_cached` sends `Content-Length: 4` with a 5-byte body; loopback servers `accept()` a fixed count so a behaviour change hangs instead of failing. | Low | Networking | Fix the header; add a timeout or `try_recv` pattern to the test servers. | Open |

## Security & Storage

| ID | Gap | Severity | Owner | Resolution | Status |
| --- | --- | --- | --- | --- | --- |
| G-16 | `Origin::parse` is `todo!()`; `CookieJar::store` and `cookies_for` are `todo!()`; `Cookie` lacks path/expiry/HttpOnly/SameSite; no CORS function; default port 0 for non-http schemes. Nothing uses `security`. | High (course goal: Canvas login) | Security & Storage | Implement per `contracts/networking.md` §`security`; un-ignore the three contract tests. | Open |
| G-17 | `storage::LocalStorage` is an unscoped in-memory map with no `clear`, no persistence, and no link to `common::ipc::Storage*` or `security::StorageKey`. | Medium | Security & Storage | Key by `StorageKey`; `handle(StorageRequest)`; `StorageBackend` trait with in-memory and file implementations. | Open |

## Browser UX

| ID | Gap | Severity | Owner | Resolution | Status |
| --- | --- | --- | --- | --- | --- |
| G-18 | `browser` is a 35-line binary that registers `print`/`fetch` and evaluates `"print()"`. Depends on all 13 crates, uses 3. No library target, so shells and root tests cannot use it. | High | Browser UX, Scrum of Scrums | `src/lib.rs` with `Engine` (one document's pipeline) and `Browser` (tabs + runtime); thin CLI `main.rs`. Unblocks G-20, G-27, G-10. | Open |
| G-19 | `chrome::TabManager::{open, close}` are `todo!()`; no id allocator, navigation history, title, or address bar; nothing uses `chrome`. | Medium | Browser UX | Implement per `contracts/ui.md`; un-ignore the three contract tests. | Open |
| G-20 | `platforms/macos` depends on no workspace crate, draws its own toolbar with disabled Back/Forward arrows, renders no `Frame`; `linux/` and `windows/` are empty; README says AppKit but code is winit. | Medium | Browser UX | Depend on `browser` once it is a library; source UI state from `chrome`. Decide ADR 0005 before adding another toolkit. | Open, blocked on G-18 |

## DevTools

| ID | Gap | Severity | Owner | Resolution | Status |
| --- | --- | --- | --- | --- | --- |
| G-21 | `devtools::Console::entries_at` is `todo!()`; nothing feeds the console; no snapshots or protocol. | Medium | DevTools | Implement filtering; consume `common::console::ConsoleMessage` (G-12); first `DomSnapshot` when the DOM read API settles. | Open |

## Suggested sequencing across teams

1. **This week:** G-25, G-26, G-28, G-22 (cleanups, any team, small PRs).
   G-01 as one coordinated PR led by HTML.
2. **Unblockers:** G-09 (JS calls), G-04 (CSS takes the real DOM), G-18
   (`browser` library), G-16 (origin + cookies). Each unblocks another team.
3. **Then:** G-05/G-06/G-08 (layout from styles), G-10/G-11 (script host),
   G-14 (net calls security), G-19/G-20 (chrome + shell), G-12/G-21 (console).
4. **Decisions to record meanwhile:** ADR 0001–0004 (drafts for 0001 and
   0002 are in `docs/adr/`).

## Known limitations of this audit

- It reads code and ran `make lint`, `make test`, and the four ignored
  contract tests named in G-25 (all pass). Other claims about behaviour come
  from reading, not execution.
- It reflects `main` at the commit above. PRs merged after that may have
  closed rows; mark them.
- Severity is a judgement call by Scrum of Scrums. Teams who disagree should
  say so in review; the point of the table is to argue about it in one place.
