# Continuous integration

Pull requests targeting `main` run the GitHub Actions workflow in
[`.github/workflows/ci.yml`](../.github/workflows/ci.yml).

The workflow has five checks:

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
