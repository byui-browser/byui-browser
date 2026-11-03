# byui-browser

The BYU-Idaho browser developed by the CSE 199R and CSE 399R classes.

An independent web browser built from scratch in Rust. See
[`docs/TECH_ARCHITECTURE.md`](docs/TECH_ARCHITECTURE.md) for architecture,
crate ownership, and collaboration rules. Team overviews live in
[`docs/TEAMS.md`](docs/TEAMS.md).

## Downloads

Pushing a version tag (`v0.1.0`, or any `v*.*.*` tag) runs the
[release workflow](.github/workflows/release.yml). It builds the native shell
for macOS, Linux, and Windows, then attaches one archive to the
[GitHub Release](https://github.com/byui-browser/byui-browser/releases) for that
tag. These links always download the latest release:

- [macOS (Apple Silicon)](https://github.com/byui-browser/byui-browser/releases/latest/download/byui-browser-macos-aarch64.tar.gz)
- [Linux (x86_64)](https://github.com/byui-browser/byui-browser/releases/latest/download/byui-browser-linux-x86_64.tar.gz)
- [Windows (x86_64)](https://github.com/byui-browser/byui-browser/releases/latest/download/byui-browser-windows-x86_64.zip)

Each archive contains a `byui-browser` program (`byui-browser.exe` on
Windows). The Linux build uses the system GTK 3 libraries, so install
`libgtk-3-0` on Debian and Ubuntu before launching it. macOS quarantines the
downloaded binary; clear that with
`xattr -d com.apple.quarantine byui-browser` before the first launch.

## Repository layout

```
.
├── Cargo.toml           # Workspace root
├── Makefile             # build / test / lint / release helpers
├── crates/
│   ├── common/          # Shared types, errors, IPC (joint ownership)
│   ├── html/            # HTML Team
│   ├── css/             # CSS Engine Team
│   ├── layout/          # Layout & Rendering Team
│   ├── paint/           # Layout & Rendering Team
│   ├── render/          # Layout & Rendering Team
│   ├── js/              # JavaScript Engine Team
│   ├── webapis/         # JS APIs (Web APIs) Team
│   ├── net/             # Networking Team
│   ├── storage/         # Security & Storage Team
│   ├── security/        # Security & Storage Team
│   ├── chrome/          # Browser UX Team
│   ├── devtools/        # Devtools Team
│   └── browser/         # Top-level binary (wires crates/processes)
├── platforms/           # Thin native shells (macOS, Windows, Linux)
├── tests/               # Integration + conformance tests
└── docs/
    ├── TECH_ARCHITECTURE.md
    ├── TEAMS.md
    ├── CI.md
    ├── PLATFORM_TESTING.md
    └── adr/             # Architecture Decision Records
```

## Prerequisites

- Rust stable (1.85+, edition 2024)

```bash
rustup update stable
```

## Build & test

```bash
make build                 # build the workspace (debug)
make run                   # run the native shell for the current OS
make test                  # unit tests across the workspace
make test html             # tests for a single crate
cargo test -p html -- --ignored   # backlog: contract tests not yet implemented
make lint                  # rustfmt check + clippy (-D warnings)
make release linux         # release build of the native shell (macos, linux, or windows)
```

The binary entry point:

```bash
cargo run -p browser
```

`make run` selects the platform shell automatically. The macOS, Linux, and
Windows shells currently render the startup HTML page, parse its CSS, and
execute its JavaScript startup script; see [`platforms/`](platforms/README.md)
for what each shell needs.

## Logging

Every crate logs through the [`log`](https://docs.rs/log) macros, and
`common::logging` prints them to standard error. Choose how much you see when
you start the browser:

```bash
cargo run -p browser-linux -- -v                          # info
cargo run -p browser-linux -- --log-level debug           # debug everywhere
cargo run -p browser-linux -- --log-level warn,net=trace  # one crate in detail
make run LOG=debug                                        # same, through make
BYUI_LOG=js::vm=trace cargo run -p browser-js             # via the environment
cargo run -p browser-linux -- --help                      # all options
```

| Level   | Shows |
| ------- | ----- |
| `error` | Failures the user notices: a page did not load, storage could not open. |
| `warn`  | Problems the browser recovered from: a skipped subresource, a missing font, a blocked request. **Default.** |
| `info`  | One line per milestone: startup, window created, navigation started and finished. |
| `debug` | Per-operation detail for developers: requests and responses, parse and layout summaries, cache hits. |
| `trace` | Everything, including tokens, DOM nodes, bytecode, and every input event and frame. |

`-vv` and `-vvv` select `debug` and `trace`; `-q` shows only errors and `-qq`
turns logging off. `--log-file <path>` appends to a file instead of standard
error. A spec's targets are crate names or module paths (`html::tokenizer`),
and the longest match wins. Levels and `-v` apply to the browser's own crates;
third-party crates (winit, reqwest, hyper, ...) stay at `warn` unless you add
`deps=<level>` or name one (`--log-level debug,hyper=trace`). New workspace
crates must be added to `WORKSPACE_CRATES` in `common::logging`; a test checks
this. See `crates/common/src/logging.rs` for the
guidance on choosing a level when you add a log line.

## Development workflow

- Trunk-based development on `main`
- Feature branches: `team/short-description` (e.g. `css/cascade-layers`)
- Before merge: `make lint` and `make test` (or `make test <crate>` for changed crates)
- Cross-crate / `common` changes need review from affected owning teams
- Record architecture decisions in [`docs/adr/`](docs/adr/)
- See [`docs/CI.md`](docs/CI.md) for pull-request checks and local equivalents.

## Status

Scaffolding only. Crates compile and declare ownership boundaries; engine
behavior is intentionally unimplemented so teams can fill in their crates.
