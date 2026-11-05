# Continuous integration

Pull requests targeting `main` run the GitHub Actions workflow in
[`.github/workflows/ci.yml`](../.github/workflows/ci.yml).

The workflow has six jobs:

- `lint` runs `cargo fmt --all -- --check` and
  `cargo clippy --workspace --all-targets -- -D warnings`.
- `test (ubuntu)`, `test (macos)`, and `test (windows)` run every workspace
  package separately with `cargo test --package <crate>`.
- `audit` runs `cargo-deny` for dependency advisories, licenses, and sources.
- `lint, test` is the required aggregate gate for the branch ruleset.

Before opening a pull request, run:

```sh
make lint
make test
```

On Windows, the equivalent commands are:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The workflow's OS matrix is important for platform crates: Windows-only code
must still compile cleanly when its test modules are built on Linux and macOS.

## Releases

Pushing a `v*.*.*` tag (for example `v0.1.0`) runs
[`.github/workflows/release.yml`](../.github/workflows/release.yml). The
workflow release-builds the native shell on each operating system and uploads
the archives to a GitHub Release:

| Operating system | Package | Archive |
| --- | --- | --- |
| macOS (Apple Silicon) | `browser-macos` | `byui-browser-macos-aarch64.tar.gz` |
| Linux (x86_64) | `browser-linux` | `byui-browser-linux-x86_64.tar.gz` |
| Windows (x86_64) | `byui-browser-windows` | `byui-browser-windows-x86_64.zip` |

`make release <macos|linux|windows>` runs the same `cargo build --locked
--release` command locally. The README links to the latest release assets.
