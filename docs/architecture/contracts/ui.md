# Contract: Chrome, platform shells, the browser binary, and DevTools

**Teams:** Browser UX (`chrome`, `browser`, `platforms/*`), DevTools
(`devtools`), with every engine team as a provider of inspection hooks.

## The composition layer: `browser`

`browser` is the only crate that depends on everything. It is **a library
with a thin binary**, not just a `main.rs`:

```rust
// browser (lib)
pub struct Browser { /* tabs, per-tab Engine, net controller, storage root, console sink */ }
impl Browser {
    pub fn new(config: BrowserConfig) -> Result<Self, BrowserError>;
    pub fn open_tab(&mut self, url: &str) -> chrome::TabId;
    pub fn navigate(&mut self, tab: chrome::TabId, url: &str) -> Result<(), BrowserError>;
    pub fn back(&mut self, tab: chrome::TabId) -> bool;
    pub fn forward(&mut self, tab: chrome::TabId) -> bool;
    pub fn tick(&mut self) -> bool;                       // run net completions + script tasks; true if more work
    pub fn render(&mut self, tab: chrome::TabId, viewport: Size, scale: f32) -> Result<render::Frame, BrowserError>;
    pub fn chrome(&self) -> &chrome::TabManager;          // for shells to draw the UI from
    pub fn console(&self) -> &devtools::Console;
}

pub struct Engine { /* one document: Document, ComputedStyles, LayoutTree, ScriptHost */ }
```

Rules:

- All wiring (load URL → `net` → `html` → `css` → `layout` → `paint` →
  `render`; `webapis::ScriptHost::new(HostServices{..})`) happens here and
  nowhere else. If a crate needs another crate it is not allowed to depend
  on, the answer is a hook that `browser` connects.
- `browser` owns the tokio runtime and the main loop's `tick`.
- The binary (`src/main.rs`) is a CLI: load a URL or file, run to idle, print
  console output and optionally write a PNG. It is the integration test
  harness and the fallback when no shell is available.

## Browser UI state: `chrome`

`chrome` is a **pure state model** with no windowing dependency. Shells draw
it; `browser` drives it.

```rust
pub struct TabId(u32);
pub struct Tab { pub id: TabId, pub title: String, pub history: NavigationHistory, pub loading: bool }
pub struct NavigationHistory { /* entries + index */ }
impl NavigationHistory { pub fn current(&self) -> Option<&str>; pub fn can_go_back(&self) -> bool; pub fn can_go_forward(&self) -> bool; pub fn push(&mut self, url: String); pub fn back(&mut self) -> Option<&str>; pub fn forward(&mut self) -> Option<&str>; }
pub struct TabManager { /* tabs, active, id allocator */ }
impl TabManager { pub fn open(&mut self, url: &str) -> TabId; pub fn close(&mut self, id: TabId) -> Option<TabId>; pub fn activate(&mut self, id: TabId); pub fn active(&self) -> Option<TabId>; pub fn tabs(&self) -> &[Tab]; }
pub struct AddressBar { pub text: String, pub editing: bool }
```

Later: bookmarks, settings, find-in-page state. All as data.

## Platform shells: `platforms/*`

A shell is the thinnest possible adapter between an OS window and `Browser`:

1. create a window, 2. on resize/redraw call `browser.render(tab, viewport,
scale)` and blit the `Frame`, 3. draw toolbar/tabs from `browser.chrome()`,
4. translate input events into `Browser` calls, 5. call `browser.tick()` when
idle.

Rules:

- A shell depends on `browser` and `render` (for `Frame`). Nothing else from
  the workspace.
- No engine logic, no UI *state*, no network. If the toolbar needs to know
  whether Back is enabled, it asks `chrome::NavigationHistory`.
- The toolkit question (winit + softbuffer everywhere vs. native per
  platform) is ADR 0005. Until decided, do not add a second windowing library.
- Each shell compiles on every OS (stub `main` elsewhere) so `cargo build
  --workspace` stays green in CI.

## DevTools: `devtools`

DevTools is a **consumer** of inspection APIs, never a dependency of engine
crates:

```rust
pub struct Console { pub fn push(&mut self, m: common::console::ConsoleMessage); pub fn entries(&self) -> &[ConsoleMessage]; pub fn filter(&self, level: ConsoleLevel) -> impl Iterator<Item = &ConsoleMessage>; }
pub struct DomSnapshot { /* built from &html::Document */ }
pub struct StyleSnapshot { /* built from &css::ComputedStyles for one NodeId */ }
pub struct LayoutSnapshot { /* built from &layout::LayoutTree */ }
pub struct NetworkLog { /* records Request/Response summaries from browser */ }
```

- Pull model for structure: `devtools` reads `&Document`, `&ComputedStyles`,
  `&LayoutTree` through their public read APIs.
- Push model for events: console messages and network events are plain
  `common` types that `browser` forwards into `devtools`.
- A protocol (serialising snapshots for a separate panel/process) is a later
  layer on top of these types, not a prerequisite for them.

## Current state (2026-10-03)

- `browser` is a 35-line binary that registers `print` and `fetch` into a
  `js::Realm`, builds a `net::RequestController`, and evaluates `"print()"`.
  It composes none of html/css/layout/paint/render/chrome/devtools despite
  depending on all of them. There is no library target. (G-18)
- `chrome::TabManager::{open, close}` are `todo!()`; there is no navigation
  history or address bar state; nothing uses `chrome`. (G-19)
- `platforms/macos` draws its own toolbar with Back/Forward arrows directly
  into a `softbuffer` surface and depends on no workspace crate.
  `platforms/{linux,windows}` are empty. Back/forward state lives in neither
  `chrome` nor the shell. (G-20)
- `devtools::Console` has its own `Level`/`LogEntry`, `entries_at` is
  `todo!()`, and nothing feeds it. (G-12, G-21)
