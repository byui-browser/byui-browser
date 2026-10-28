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
[GitHub Release](https://github.com/byui-cse/byui-browser/releases) for that
tag. These links always download the latest release:

- [macOS (Apple Silicon)](https://github.com/byui-cse/byui-browser/releases/latest/download/byui-browser-macos-aarch64.tar.gz)
- [Linux (x86_64)](https://github.com/byui-cse/byui-browser/releases/latest/download/byui-browser-linux-x86_64.tar.gz)
- [Windows (x86_64)](https://github.com/byui-cse/byui-browser/releases/latest/download/byui-browser-windows-x86_64.zip)

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
