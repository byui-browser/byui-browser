# byui-browser

The BYU-Idaho browser developed by the CSE 199R and CSE 399R classes.

An independent web browser built from scratch in Rust. See
[`docs/TECH_ARCHITECTURE.md`](docs/TECH_ARCHITECTURE.md) for architecture,
crate ownership, and collaboration rules. Team overviews live in
[`docs/TEAMS.md`](docs/TEAMS.md).

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
make release linux         # release build for macos | linux | windows
```

The binary entry point:

```bash
cargo run -p browser
```

`make run` selects the platform shell automatically. The macOS shell currently
renders the startup HTML page, parses its CSS, and executes its JavaScript
startup script. Linux and Windows shells are not implemented yet, so the
command reports that limitation instead of running the wrong binary.

## Development workflow

- Trunk-based development on `main`
- Feature branches: `team/short-description` (e.g. `css/cascade-layers`)
- Before merge: `make lint` and `make test` (or `make test <crate>` for changed crates)
- Cross-crate / `common` changes need review from affected owning teams
- Record architecture decisions in [`docs/adr/`](docs/adr/)

## Status

Scaffolding only. Crates compile and declare ownership boundaries; engine
behavior is intentionally unimplemented so teams can fill in their crates.
