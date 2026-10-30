# Windows shell

A minimal native Windows window written in Rust using `winit`. The window,
toolbar, and rendering code are shared with the macOS and Linux shells through
[`platforms/shell`](../shell); this crate only supplies Windows font paths and
keeps the native Windows titlebar and frame.

## Run

Requires Windows, Rust 1.85 or newer, and the Visual C++ build tools. From the
repository root:

```powershell
cargo run -p byui-browser-windows
```

`make run` selects this shell automatically on Windows.

The shell currently shows the built-in `byui://welcome` page. The address field
is interactive: click it or press `Ctrl+L`, edit text, press `Enter` to submit,
or `Escape` to cancel. `Ctrl+R` triggers the existing repaint-only Reload
action. Only the built-in welcome URL is loadable at this stage; other submitted
URLs remain visible without pretending that a network navigation succeeded.
Tabs, Back, Forward, and the menu remain staged until their browser services
exist.

Windows supplies the native titlebar, resize borders, and system window
buttons. The normal window uses Windows' rounded frame, while a maximized
window uses the system's square maximized frame.
