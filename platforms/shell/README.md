# Shared shell

The `browser-shell` library holds everything the native desktop shells have in
common: the `winit` event loop, the software-painted toolbar and window
controls (`tiny-skia` and `ab_glyph`, presented through `softbuffer`), and the
in-process welcome-page demo built on the layout, paint, and render crates.

A platform crate describes its window through `browser_shell::Platform` (how
to hide the native titlebar, which font files to try for toolbar text, and
whether the shell must draw its own resize handles) and calls
`browser_shell::run`. See [`macos/`](../macos) and [`linux/`](../linux).

On operating systems without a supported shell the crate still builds and runs
its drawing tests, so workspace checks pass everywhere.
