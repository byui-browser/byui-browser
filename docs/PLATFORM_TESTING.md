# Platform shell testing

Run the shell on each native operating system after platform-specific changes:

| Platform | Command | Verify |
| --- | --- | --- |
| macOS | `cargo run -p browser-macos` | Native traffic lights, toolbar inset, resize hover state, drag empty toolbar space, and zoom toggle |
| Linux | `cargo run -p browser-linux` | Native titlebar, X11/Wayland startup, resize behavior, toolbar hit testing, and zoom toggle |
| Windows | `cargo run -p byui-browser-windows` | Native titlebar, rounded normal frame, square maximized frame, resize behavior, toolbar hit testing, and zoom toggle |

For every shell, confirm the welcome page renders, Reload responds, disabled
Back and Forward do not drag or maximize the window, and the address field
shows the text cursor when hovered. Navigation and address editing are not
expected until the browser engine exposes those APIs.
