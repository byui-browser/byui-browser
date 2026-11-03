# Linux shell

A minimal native Linux window written in Rust using `winit`, supporting both
X11 and Wayland. It shows the same chrome and fixed HTML demo as the macOS
shell; the window, toolbar, and rendering code is shared in
[`platforms/shell`](../shell).

## Run

Requires Rust 1.85 or newer and a desktop session. From the repository root:

```sh
make run
```

On Linux, `make run` selects this shell automatically. The direct command
`cargo run -p browser-linux` remains available for platform-only development.

winit opens a Wayland window when `WAYLAND_DISPLAY` is set and an X11 window
otherwise. The X11 and Wayland client libraries are loaded at runtime, so no
development packages are needed to build; at runtime the shell needs the
libraries any desktop already has (`libxkbcommon`, plus `libX11`, `libXcursor`,
`libXrandr`, `libXi` and `libxkbcommon-x11` for X11, or `libwayland-client` for
Wayland). To force one backend, run with only that display set, for example
`WAYLAND_DISPLAY= cargo run -p browser-linux` for XWayland.

## What you see

A 1120 × 720 window titled **BYUI Browser** opens with the desktop's own
titlebar and frame above a unified toolbar (52 logical pixels high) and the
page shown as a rounded card inset from the window frame. The window
manager's buttons close, minimize, and maximize the window, and its borders
resize it, exactly as for any other application. On Wayland compositors
without server-side decorations (such as GNOME's) winit draws an
Adwaita-style titlebar instead. Dragging empty toolbar space also moves the
window, and double-clicking it toggles maximized, like Safari's toolbar.

From left to right the toolbar shows Back, Forward, Reload, a centered address
field labelled `byui://welcome`, New Tab, and a menu button. Back and Forward
are drawn disabled, Reload re-renders the welcome page, and the address field
supports staged editing through click/`Ctrl+L`, Enter, Escape, and Backspace.
New Tab and the menu remain visual placeholders.

Toolbar text uses the desktop's default sans-serif font, found through
`fc-match`, falling back to Noto Sans, DejaVu Sans, or Liberation Sans at their
usual paths. If none is found the toolbar is drawn without text.

## Limitations

- The chrome always uses the light palette: `winit` does not report the
  desktop's dark-mode preference on X11 or Wayland.

On other operating systems, the executable only reports that Linux is required.
This lets workspace checks run without building a Linux window on those systems.
