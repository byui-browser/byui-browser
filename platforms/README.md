# Platform shells

Thin native shells that host the browser UI. The engine produces pixels and events; shells stay small.

| Directory  | Platform                 | Owning team | Status          |
|------------|--------------------------|-------------|-----------------|
| `shell/`   | Shared (`winit`)         | Browser UX  | Window, toolbar, and page rendering used by every shell |
| `macos/`   | macOS (AppKit via winit) | Browser UX  | Runs: `cargo run -p browser-macos` |
| `linux/`   | X11 and Wayland via winit| Browser UX  | Runs: `cargo run -p browser-linux` |
| `windows/` | Win32 / WinUI            | Browser UX  | Not implemented |

`make run` picks the shell for the current operating system. Each platform
crate only describes its window (how to hide the native titlebar, which system
font to use) and calls `browser_shell::run`; the chrome and the welcome-page
demo live once in `shell/`. Create a shell subdirectory when starting platform
work. Do not put engine logic here.
