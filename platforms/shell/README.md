# Shared shell

The `browser-shell` library holds everything the native desktop shells have in
common: the `winit` event loop, the software-painted toolbar and window
controls (`tiny-skia` and `ab_glyph`, presented through `softbuffer`), and the
in-process welcome-page demo built on the layout, paint, and render crates.

Window decorations are always the operating system's own. A platform crate
describes its window through `browser_shell::Platform` (window attributes such
as a transparent titlebar, which font files to try for toolbar text, and how
much of the toolbar's left edge the system's window buttons cover) and calls
`browser_shell::run`. See [`macos/`](../macos) and [`linux/`](../linux).

On operating systems without a supported shell the crate still builds and runs
its drawing tests, so workspace checks pass everywhere.

The shared app owns `chrome::BrowserState` and records `byui://welcome` once
its document and resources load successfully. The loaded document is retained
separately from its rendered pixmap, so repainting, resizing, and the current
Reload action do not add visits. History lives in memory for this app session;
there is no history UI or persistence yet.

Future successful navigation should pass its loaded document and URL through
`App::page_loaded`. Navigation itself is not implemented, and the current
renderer still supports only the welcome demo. Reload fetches the saved current URL on a background task and prints its URL,
status, and response body (or an error) to the launching terminal. It bypasses
the HTTP response cache and leaves the displayed document and history unchanged.
Internal `byui://welcome` uses the embedded page provider, not HTTP or disk.
HTTP and HTTPS use the Networking team's transport; address editing and navigation
remain unimplemented, so the current UI refreshes the welcome page.

Run `cargo test -p browser refresh::tests` to verify fresh HTTP requests,
internal pages, saved URLs, error reporting, and unchanged history.

Run `cargo test -p browser-shell app::tests` to verify visit recording and
that redraw/resize/Reload work does not append history. Run
`cargo run -p browser-macos` on macOS or `cargo run -p byui-browser-windows`
on Windows to check that the existing welcome page and toolbar still work.
