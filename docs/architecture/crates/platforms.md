# Crates: `platforms/*`

**Purpose:** thin native shells. One per OS. A shell opens a window, asks
`browser` for a `Frame`, blits it, draws toolbar/tabs from `chrome` state,
and forwards input events. Nothing else.
**Maintained by:** Browser UX Team.
**Contract:** [`../contracts/ui.md`](../contracts/ui.md) §Platform shells.

## Dependencies

- May depend on: `browser`, `render` (for `Frame`), and one windowing
  library per ADR 0005.
- Depended on by: nothing.
- Must never depend on: engine crates directly, `chrome` directly (go
  through `browser.chrome()`), or `net`.

## Intended public contract

Shells have no public API. Each is a `[[bin]]` that compiles on every OS
(stub `main` elsewhere) so `cargo build --workspace` is green in CI.

The loop: create window → on redraw `browser.render(active_tab, viewport,
scale)` → blit `Frame` → draw chrome from `browser.chrome()` → on input call
`browser.navigate/back/forward/open_tab` → on idle `browser.tick()` and
request a redraw if it returns `true`.

## Current state (2026-10-03)

- `platforms/macos` (`browser-macos`, about 230 lines): a `winit` 0.30 +
  `softbuffer` 0.4 window titled "BYUI Browser", 640×480, with a hand-drawn
  48 px toolbar and two disabled Back/Forward arrow buttons in
  `toolbar.rs`. Handles resize and scale factor. **Depends on no workspace
  crate**; does not consult `chrome` or render any `Frame`. Compiles on
  Linux via a stub `main` (G-20). The README table says AppKit; the code
  uses winit.
- `platforms/linux`, `platforms/windows`: empty `.gitkeep` only.
- Toolbar drawing has 4 unit tests that run on every OS.

## Gaps owned by this crate

G-20: once `browser` is a library (G-18), depend on it, blit its `Frame`
under the toolbar, and source Back/Forward enablement from
`chrome::NavigationHistory`. Decide ADR 0005 before adding a second
toolkit; the current winit + softbuffer stack is cross-platform, which may
make `linux/` and `windows/` a cfg-gate rather than separate crates.

## Read next

`contracts/ui.md`, `crates/browser.md`, `crates/chrome.md`.
