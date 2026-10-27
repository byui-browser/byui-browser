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

### Page context menu

Right-click inside the page card to open the shared desktop context menu.
The menu lists **Back**, **Forward**, **Reload**, and **Inspect (page source)**.
Back and Forward are visibly disabled placeholders: they do not highlight,
respond to clicks, or navigate, even when history entries exist. Clicking them
leaves the popup open. Navigation controls are not connected yet.
Choose **Reload** to use the same asynchronous saved-URL fetch as the toolbar.
The URL, status, and body appear in the launching terminal. History is unchanged.
Click outside the popup or press Escape to dismiss it. Resize and focus loss also
close it; the cached page is restored without re-running scripts on dismissal.
The popup is clamped to the window and supports display scaling. The fourth
item, **Inspect (page source)**, opens a read-only **Page source (loaded HTML)**
panel using a snapshot of the loaded document. It does not fetch, execute HTML,
select elements, or show a live DOM. Scroll with the mouse wheel, Up/Down, or
Page Up/Page Down; Home/End jump to the beginning/end. Small trackpad deltas
accumulate, long lines wrap, and the viewer prefers a system monospace font.
Line numbers, a wrapped-row count, and a scroll-position indicator help with
long documents (the indicator is not draggable). Wrapped text is cached until
width changes, and resizing preserves the reading position where possible.
Click the visible **Close** button, press Escape, or click the toolbar to close
the panel and restore the cached page. Keyboard menu-item navigation and the
three-dot menu remain future work. macOS and Windows use the same implementation.

For a manual demo, run `cargo run -p browser-macos`, right-click the welcome page,
click Reload, and look for `Refresh byui://welcome`, `Status: 200`, and the HTML.
Try the bottom-right corner, outside-click and Escape dismissal, and resizing;
no popup pixels should remain after dismissal. On Windows, run
`cargo run -p byui-browser-windows` and repeat the same steps.

To demonstrate Inspect, right-click the welcome page and select **Inspect (page
source)** below Reload. Point out the “Page source (loaded HTML)” heading and
welcome-page HTML. Press Escape to return to the page, then demonstrate that
Reload still prints its results in the terminal. This is source viewing, not a
full element inspector. Windows UI behavior still needs a native manual check.

To test scrolling, shrink the window near its minimum size while source is open.
Try wheel/trackpad, Up/Down, Page Up/Down, and Home/End. Resize wider and taller;
the source panel and Close button should remain inside the window. The source
viewer tests cover long lines and hundreds of HTML lines. This feature adds no
navigation or startup-URL support.
