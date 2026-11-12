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

The shared app owns `chrome::BrowserState` and records a successful navigation
after its document and resources load. The loaded document is retained
separately from its rendered pixmap, so repainting, resizing, and Reload do not
add visits. History lives in memory for this app session; the toolbar Back and
Forward controls traverse it without adding new visits. The context-menu rows
remain disabled until they are wired to the same history service.

The address field accepts the built-in `byui://welcome` page and HTTP/HTTPS
addresses. Navigation runs through the browser loader on a background task,
shows a loading indicator, and replaces the page with the loaded document or a
built-in error page. Reload fetches the current address again, bypasses the
response cache, and preserves history. Internal `byui://welcome` uses the
embedded page provider, not HTTP or disk.

The address field is an editable shell surface: click it or press `Ctrl+L`,
use normal text editing plus Backspace, press Enter to navigate, or Escape to
restore the previous value. `Ctrl+R` triggers Reload. Submitted HTTP and HTTPS
URLs remain in the address field while loading and are replaced by the loaded
page or an error page when the request completes. Tabs, Back, Forward, and the
menu remain staged until their browser services exist.

Run `cargo test -p browser refresh::tests` to verify fresh HTTP requests,
internal pages, saved URLs, error reporting, and unchanged history.

Run `cargo test -p browser-shell app::tests` to verify visit recording and
that redraw/resize/Reload work does not append history. Run
`cargo run -p browser-macos` on macOS or `cargo run -p byui-browser-windows`
on Windows to check that the existing welcome page and toolbar still work.

### Page context menu

Right-click inside the page card to open the shared desktop context menu.
The menu lists **Back**, **Forward**, **Reload**, **View Page Source**, and **Inspect**.
Back and Forward are enabled when session history allows the action. They
traverse successful visits without adding a new history entry, and stay
disabled at the ends of the history list.
Choose **Reload** to use the same asynchronous saved-URL fetch as the toolbar.
The URL, status, and body appear in the launching terminal. History is unchanged.
Click outside the popup or press Escape to dismiss it. Resize and focus loss also
close it; the cached page is restored without re-running scripts on dismissal.
The popup is clamped to the window and supports display scaling. The fourth
item, **View Page Source**, opens a read-only **Page source (loaded HTML)**
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

To demonstrate View Page Source, right-click the welcome page and select **View Page Source** below Reload. Point out the “Page source (loaded HTML)” heading and
welcome-page HTML. Press Escape to return to the page, then demonstrate that
Reload still prints its results in the terminal. This is source viewing, not a
full element inspector. Windows UI behavior still needs a native manual check.

To test scrolling, shrink the window near its minimum size while source is open.
Try wheel/trackpad, Up/Down, Page Up/Down, and Home/End. Resize wider and taller;
the source panel and Close button should remain inside the window. The source
viewer tests cover long lines and hundreds of HTML lines. This feature adds no
navigation or startup-URL support.

### Inspect: basic Elements panel

The fifth right-click action, **Inspect**, opens a rectangular bottom panel
inside the window. The upper webpage stays visible. **Elements — Basic
inspector** displays an indented tree from the current live document when the
page session is available, with a loaded-HTML fallback. It remains read-only:
text, comments, attributes, editing, and element picking are not currently
shown. The panel closes when navigation replaces the displayed document.

The panel also includes session-local **Network** and **Console** tabs. Network
shows the document and linked stylesheet/script resources discovered for the
page; Console receives `console.log`, `console.warn`, `console.error`, and the
legacy `print()` bridge. Delete clears the active log, and the native DevTools
window opened with Ctrl+Shift+H can be closed with Escape.

Scroll over the panel, or use Up/Down, Page Up/Down, Home/End while the address
field is not focused. Click **×** or press Escape to close it. Resize the window
to check that the dock and close control remain inside it. Missing/empty HTML
shows a diagnostic rather than crashing. The basic view is bounded to 1 MiB of
HTML and 20,000 element entries.

View Page Source remains the separate full HTML viewer with line numbers,
wrapping, scrolling, Close, and Escape. Both views keep their own state: open
Inspect, then right-click the upper webpage and choose View Page Source. Escape
closes the source overlay first and reveals the still-open Elements panel;
Escape again closes Elements. Clicking inside Elements never dispatches a
click to the webpage underneath. Reload and the toolbar three-dot menu are
unchanged.

### Audit Compatibility: what this page uses that the engine cannot do yet

The Inspect dock has a second tab, **Compatibility**. Open it by clicking the
tab in the dock header, or choose **Audit Compatibility** (the last item in the
page's right-click menu). It lists every feature the current page uses that
does not work in this engine, grouped by area: script execution, JavaScript
syntax, JavaScript and Web APIs, HTML, CSS parsing, properties, values,
at-rules, and selectors. Each row shows how often the page uses the feature,
where it first appears (for example `app.js:17` or `site.css:4`), and how the
audit knows:

- **probed** (red): the engine was tested and the feature did not work.
  JavaScript probes run tiny scripts in a page realm set up exactly like a
  real page's; CSS and HTML probes render a small fixture through the real
  `css`, `layout`, and `paint` crates and compare display lists.
- **observed** (orange): seen in this load, such as a failed `<script src>`, a
  `type="application/ld+json"` block being run as JavaScript, or the error
  that stopped the page's scripts.
- **by name** (yellow): an instance member such as `.forEach` matched by
  name, so the page might not actually be affected.
- **documented** (gray): cannot be probed statically, but the engine documents
  the limitation (for example `:hover` never matches).

Nothing comes from a hard-coded "unsupported" list. When a team ships a
feature, its probe starts passing and it drops out of the report. Features no
static probe can check (such as `cursor` or `transition`) are listed under
**Not verified**. The audit runs on a background thread the first time the
tab is shown and reuses probe results for the rest of the session. The
detection and probe logic lives in `devtools::compat`.

### Loading indicator

The address field reserves a compact **Loading...** label while the newest
navigation or Reload fetch is pending. The URL, caret and toolbar controls remain
usable. The label uses the existing light/dark palette and scales with the window.
Each loading operation has an ID separate from navigation IDs: an older completion
cannot hide a newer operation's indicator. Success and failure both clear it;
unavailable loaders and missing saved URLs do not leave it stuck.

Reload still uses the existing console-only Refresh fetch and does not replace
the document or add history. Its task handle is checked on the UI thread at
40 ms intervals only while pending; no network work blocks the event loop.
The label tracks fetching/loading, not completion of all page-script execution.
Immediate embedded welcome loads may finish before a visible loading frame.

On macOS run `cargo run -p browser-macos`. Submit a slow local HTTP URL and check
that Loading... appears, then disappears on success or failure. Start a second
load before the first finishes: the first result must not hide the second label.
Try toolbar Reload, right-click Reload and Command-R; terminal output and history
behavior remain unchanged. Check address editing, resizing, system appearance,
View Page Source and Inspect. Fast responses may finish too quickly to see the
label. Native Windows/Linux interaction still requires manual verification.

### Find in Page — search and highlighting

Press **Command-F** on macOS or **Ctrl-F** on Windows/Linux to open and focus
Find below the toolbar. Type a query; the counter shows the selected match and
total (for example `1/3`, or `0/0` when there are no results). Use the up/down
buttons, Enter for Next, or Shift-Enter for Previous. Navigation wraps at both
ends and scrolls to the selected text fragment's real CSS position, clamped to
the document's scroll range. Escape or × closes Find. Opening Find reserves a
row above the page, so it does not cover the text. Clicking the page or address
field releases query focus; click the query or press Command/Ctrl-F to refocus
and select its contents. Arrow keys, Home/End, Backspace/Delete, select-all and
paste work in the query. Long queries scroll horizontally within their field.

Search uses text in the current layout, including offscreen laid-out text;
it does not search raw HTML, source panels, address text, scripts or styles.
Adjacent inline text fragments join, whitespace collapses, and wrapped lines
join with a space. Separate block contexts and atomic inline boundaries stay
separate. Matches are non-overlapping substrings. Unicode matching uses Rust's
locale-independent lowercase conversion, not full case folding or Unicode
normalization: `CAFÉ` matches `café`, but `Straße` does not match `STRASSE`, and
composed/decomposed accents remain distinct. Whitespace-only queries have no
matches. Counts describe the engine's layout, not an independently queried live
JavaScript DOM. Layout changes, resizing, zoom and new documents rebuild the
index; the selected match index is preserved where possible and clamped.

Find does not implement full browser/IME search-field editing, regex, whole-word
options, or locale-sensitive search. The bottom inspector remains independent and can obscure lower page content; close
it when checking results near the bottom. Source and context menus retain their
own Escape behavior. No extra request is made and no history is recorded by Find.

Manual macOS check: run `cargo run -p browser-macos`, press Command-F and search
`welcome` on `byui://welcome`. Expect `1/1`. Try `o` for repeated results, Enter,
Shift-Enter, an absent word, then Escape and ×. Load a long page containing repeated
paragraphs and inline spans, search a phrase, and cycle to offscreen results.
Resize, zoom, navigate to another page, and check updated counts. Test query typing
versus Command-L/address typing, existing Reload and right-click actions, Source,
Elements and Compatibility, and both system appearances. Windows/Linux use the
same shell implementation but still need native manual interaction checks.

Matches now have measured backgrounds behind the text: ordinary matches use a
translucent yellow/gold and the selected match uses orange, with theme-specific
colors. Original UTF-8 ranges are retained across fragments. Measurements use the
same font advances and CSS bounds as layout and rendering, never fixed character
counts. Wrapped and cross-inline phrases get one rectangle per matched fragment
portion; synthetic wrap spaces get no invented rectangle. Highlights are paint
commands only and do not change hit testing. Closing Find removes them.

Unicode matching now requires complete extended grapheme clusters, including
clusters split across inline elements. Lowercase expansions map back to the whole
original cluster: `i` alone does not match `İ`, while `i` followed by combining dot
matches and highlights the original `İ`. A base character alone does not match
part of a base-plus-combining-mark cluster. Counts and highlights use the same
accepted matches. Canonical normalization and full case folding remain unsupported.
The engine itself does not shape ligatures or complex scripts and uses one page
font; highlights follow that existing renderer, not a more capable browser's text.
Rectangles cover measured advance cells, not exact glyph ink contours, so italic
overhangs and zero-advance marks can extend outside a cell as with text selection.

Shared Paint integration: `paint_document_scrolled_with_text_backgrounds` is an
optional backward-compatible hook that inserts supplied CSS-pixel rectangles
before each text fragment. Empty backgrounds preserve normal paint output. Search
policy, Unicode mapping, font measurement and colors stay in the shared shell;
Layout, Rendering and Networking are unchanged. The shell directly depends on
already-locked `unicode-segmentation` for cluster boundaries. Coordinate the Paint
API addition with Layout/Rendering and Scrum of Scrums before submitting.

Manual highlighting checks: search `welcome`, then `o`, on the welcome page.
Check yellow ordinary matches, orange selected matches, Enter/Shift-Enter and the
arrow buttons. Search inline/wrapped phrases on a long page; resize, zoom and
scroll while checking alignment. Clear the query or close Find and check that all
highlights disappear. Check webpage clicks and both system appearances. Native
Windows/Linux visual checks remain pending.
