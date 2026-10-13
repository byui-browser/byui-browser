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

A 1120 × 720 window titled **BYUI Browser** opens with a unified toolbar
(52 logical pixels high) and the page shown as a rounded card inset from the
window frame. The desktop's decorations are hidden so the toolbar spans the
whole window, and the shell draws and handles the window controls itself:
close quits, minimize iconifies the window, and zoom (or double-clicking empty
toolbar space) toggles maximized. Drag empty toolbar space to move the window,
and drag any window edge or corner to resize it. The window controls turn gray
when the window loses focus.

From left to right the toolbar shows Back, Forward, Reload, a centered address
field labelled `byui://welcome`, New Tab, and a menu button. Navigation is not
implemented yet: Back and Forward are drawn disabled, Reload re-renders the
welcome page, and the address field, New Tab, and menu only show hover and
press feedback.

Toolbar text uses the desktop's default sans-serif font, found through
`fc-match`, falling back to Noto Sans, DejaVu Sans, or Liberation Sans at their
usual paths. If none is found the toolbar is drawn without text.

## Limitations

- The chrome always uses the light palette: `winit` does not report the
  desktop's dark-mode preference on X11 or Wayland.
- The window has no drop shadow or border beyond what it paints itself, since
  the compositor's decorations are hidden.

On other operating systems, the executable only reports that Linux is required.
This lets workspace checks run without building a Linux window on those systems.
