# macOS shell

A minimal native macOS window written in Rust using `winit`. This standalone
shell displays a fixed HTML demo using the project's layout, paint, and render APIs.

## Run

Requires macOS, Rust 1.85 or newer, and the Xcode Command Line Tools
(`xcode-select --install`).
From the repository root:

```sh
make run
```

On macOS, `make run` selects this shell automatically. The direct command
`cargo run -p browser-macos` remains available for platform-only development.

A 1120 × 720 window titled **BYUI Browser** opens with a unified, Safari-style
toolbar (52 logical pixels high) and the page shown as a rounded card inset
from the window frame. The chrome follows the system light or dark appearance.

The native titlebar is hidden, so the shell draws and handles the window
controls itself: close quits, minimize sends the window to the Dock, and zoom
(or double-clicking empty toolbar space) toggles the zoomed size. Drag empty
toolbar space to move the window. The window controls turn gray when the window
loses focus.

From left to right the toolbar shows Back, Forward, Reload, a centered address
field labelled `byui://welcome`, New Tab, and a menu button. Navigation is not
implemented yet: Back and Forward are drawn disabled, Reload re-renders the
welcome page, and the address field, New Tab, and menu only show hover and
press feedback.

The chrome is drawn with `tiny-skia` (anti-aliased shapes) and `ab_glyph`
(text in the macOS system font, loaded from `/System/Library/Fonts`), then
presented through `softbuffer` and `winit`.

## Rendered content demo

The same command above displays **WELCOME TO BYUI BROWSER!** in a light-blue
block at the top of the page card, with white content beneath it. The startup page is
defined as HTML, has a CSS stylesheet parsed by the CSS crate, and runs a
JavaScript startup script through the JS crate. CSS cascade/layout and DOM
script bindings are not connected yet, so the current renderer uses its
first-slice block styling and the script is validated for execution.

The shell parses the shared welcome document from the browser crate and calls
`layout_tree`, `paint_document`, and `Compositor::compose_paint`. It converts the
returned RGBA frame to the window buffer, scaled for the display. It never reads
`target/layout-test-output/simple-element.png`; running the layout test first is
not required. This is an in-process demo of the current basic renderer, without
navigation, full CSS support, or renderer-process integration.

On other operating systems, the executable only reports that macOS is required.
This lets workspace checks run without building a macOS window on those systems.
