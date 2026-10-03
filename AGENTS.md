# Workspace guidance

## Architecture

Before changing code, read `docs/architecture/README.md`. It is a short
router: find your task in its table and load only the documents it lists
(the crate document for every crate you touch is always one of them). Do not
read the whole `docs/architecture/` tree for a single task.

If a crate document's "Current state" section no longer matches the code,
update it in the same PR. If a change would break an invariant listed in
that README or a signature in `docs/architecture/contracts/`, stop and raise
it rather than working around it; the fix is an ADR or a Scrum of Scrums
decision. Known deviations are tracked in
`docs/architecture/gap-report-2026-10.md`; do not build further on a gap
without checking its row.

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
