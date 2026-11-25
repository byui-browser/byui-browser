# Windows shell

A native Windows window written in Rust using `winit`. The window,
toolbar, and rendering code are shared with the macOS and Linux shells through
[`platforms/shell`](../shell); this crate only supplies Windows font paths and
keeps the native Windows titlebar and frame.

## Run

Requires Windows, Rust 1.88 or newer, and the Visual C++ build tools. From the
repository root:

```powershell
cargo run -p byui-browser-windows
```

`make run` selects this shell automatically on Windows.

The shell starts on the built-in `byui://welcome` page. The address field is
interactive: click it or press `Ctrl+L`, edit text, press `Enter` to navigate,
or `Escape` to cancel. HTTP and HTTPS addresses are loaded asynchronously
through the browser loader, with a loading indicator and a built-in error page
for failed requests. `Ctrl+R` reloads the current address without adding a
history visit. The toolbar Back and Forward buttons traverse successful
session history without adding visits. Tabs, appearance settings, page source,
site information, Find in Page, and the shared DevTools dock are available
through the same shell implementation used by macOS and Linux.

Windows supplies the native titlebar, resize borders, and system window
buttons. The shell explicitly requests rounded corners for a restored window
and square corners while maximized; Windows versions that do not support the
corner preference safely fall back to their system behavior.

## Windows verification checklist

- [x] Launch the Windows shell and render the built-in welcome page.
- [x] Restore the normal window and confirm the system frame remains rounded.
- [x] Maximize and restore it; maximized corners are square and restored
  corners should be rounded.
- [x] Close the final window cleanly without a tab-strip teardown panic.
- [x] Visit two addresses, then verify Back and Forward enable and traverse them.
- [x] Use `Ctrl+R` and confirm Reload does not add another history visit.
- [ ] Open and close tabs, switch tabs, and verify each tab keeps its own URL and
  history.
- [ ] Toggle System, Light, and Dark appearance and restart to confirm the choice
  persists.
- [ ] Use `Ctrl+F` to search the live page, cycle matches with Enter and
  Shift+Enter, then dismiss Find with Escape.
- [ ] Open Page Source, Site Information, and Inspect from the page menu; confirm
  each overlay closes cleanly and restores the underlying page.
- [x] Press `Ctrl+Shift+H`; switch Elements, Network, Performance, and Console,
  press Delete to clear the active log, and close DevTools.
- [ ] Open DevTools again after navigation and confirm the live document and new
  request/console entries are shown.
- [ ] Repeat the checks at Windows display scaling above 100%.

Automated verification for the current Windows sprint branch:

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets -- -D warnings`
- [x] `cargo check -p byui-browser-windows`
- [x] `cargo test -p browser-shell --lib` (194 tests)
- [x] `git diff --check`
