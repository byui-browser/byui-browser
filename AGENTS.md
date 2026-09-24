# Workspace guidance

## Public Rust APIs

Every crate-level public API must have Rust documentation. Public types,
fields, functions, methods, enum variants, and constants should document their
purpose, units, ownership, and how callers use them where applicable.

When a feature is intentionally partial, document the limitation next to the
API it affects and add a focused regression test for the supported behavior.

Run `cargo fmt --all`, the relevant `cargo test` commands, and
`cargo clippy --all-targets -- -D warnings` before opening a pull request.
