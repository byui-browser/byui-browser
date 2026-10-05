# macOS shell

A minimal native macOS window written in Rust using `winit`. This standalone
shell does not connect to the browser engine yet.

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
that the toolbar stays full-width. Back (left arrow) and Forward (right arrow) appear at the left of the toolbar.
Both are disabled placeholders: clicking them or pressing keys does not navigate.
Navigation history and actions are not implemented yet.

The shell uses `softbuffer` to display the toolbar background with `winit`.

On other operating systems, the executable only reports that macOS is required.
This lets workspace checks run without building a macOS window on those systems.
