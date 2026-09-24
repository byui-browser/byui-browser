# macOS shell

A minimal native AppKit window written in Swift. This standalone shell does not
connect to the Rust engine yet.

## Run

Requires macOS and Swift 6 or newer, included with Xcode 16+ or its Command Line
Tools (`xcode-select --install`).
From the repository root:

```sh
mkdir -p target
xcrun swiftc -swift-version 6 platforms/macos/main.swift -o target/byui-browser-macos
./target/byui-browser-macos
```

A 640 × 480 window titled **BYUI Browser** opens. Close the window using the red
close button to quit. The executable is placed in the existing ignored `target/`
directory.
