# Workspace guidance

## Public Rust APIs

Every crate-level public API must have Rust documentation. Public types,
fields, functions, methods, enum variants, and constants should document their
purpose, units, ownership, and how callers use them where applicable.

When a feature is intentionally partial, document the limitation next to the
API it affects and add a focused regression test for the supported behavior.

Run `cargo fmt --all`, the relevant `cargo test` commands, and
`cargo clippy --all-targets -- -D warnings` before opening a pull request.

## Agent skills

Shared agent skills live in `.agents/skills/<name>/SKILL.md` and are the single
source of truth for every agent (Codex, Claude Code, and others). Claude Code
loads them through thin stubs in `.claude/skills/`; when adding a skill, add a
matching stub there with the same `name` and `description`.

- `open-pr`: validate changes and open a draft pull request against `main`.
- `fix-ci`: diagnose failing PR checks and apply focused fixes.
- `fix-merge-conflicts`: resolve merge conflicts and re-validate the build.

Every AI-assisted pull request ends its description with a disclosure line
naming the agent, for example `_Opened with the help of Codex._`
