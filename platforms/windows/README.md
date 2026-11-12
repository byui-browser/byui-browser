# Windows shell

A minimal native Windows window written in Rust using `winit`. The window,
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
session history without adding visits; tabs and the menu are still staged.

Windows supplies the native titlebar, resize borders, and system window
buttons. The normal window uses Windows' rounded frame, while a maximized
window uses the system's square maximized frame.

## Windows verification checklist

- Resize the normal window and confirm the system frame remains rounded.
- Maximize and restore it; maximized corners should be square and restored
  corners should be rounded.
- Visit two addresses, then verify Back and Forward enable and traverse them.
- Use `Ctrl+R` and confirm Reload does not add another history visit.
- Press `Ctrl+Shift+H`; switch Elements, Network, and Console, scroll each
  view, press Delete to clear the active log, and press Escape to close.
- Open DevTools again after navigation and confirm the live document and new
  request/console entries are shown.
- Repeat the checks at Windows display scaling above 100%.
