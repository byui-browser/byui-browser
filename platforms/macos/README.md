# macOS shell

A minimal native macOS window written in Rust using `winit`. This standalone
shell displays a fixed HTML demo using the project's layout, paint, and render APIs.

## Run

Requires macOS, Rust 1.85 or newer, and the Xcode Command Line Tools
(`xcode-select --install`).
From the repository root:

```sh
cargo run -p browser-macos
```

A 640 × 480 window titled **BYUI Browser** opens. Close the window using the red
close button to quit. A light-gray toolbar, 48 logical pixels high, sits
above the white content area with a thin divider. Resize the window to check
that the toolbar stays full-width. Back (left arrow), Forward (right arrow), and
Reload (circular arrow) appear in that order at the left of the toolbar.
All three are disabled placeholders: clicking them or pressing keys does not
navigate or reload a page. Navigation history and reload actions are not
implemented yet.

The shell uses `softbuffer` to display the toolbar background with `winit`.

## Rendered content demo

The same command above displays **HELLO, BROWSER!** in a light-blue block below
the toolbar, with white content beneath it. Resize the window to check that the
page fits the content area while the toolbar remains unchanged.

The shell parses `<div>Hello, browser!</div>` and calls `layout_tree`,
`paint_document`, and `Compositor::compose_paint`. It converts the returned RGBA
frame to the window buffer, scaled for the display. It never reads
`target/layout-test-output/simple-element.png`; running the layout test first is
not required. This is an in-process demo of the current basic renderer, without
navigation, full CSS support, or renderer-process integration.

On other operating systems, the executable only reports that macOS is required.
This lets workspace checks run without building a macOS window on those systems.
