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
close button to quit.

On other operating systems, the executable only reports that macOS is required.
This lets workspace checks run without building a macOS window on those systems.
