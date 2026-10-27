## Summary

<!-- Why this change is needed, then what it does, in 1–3 sentences.
     Link the issue or ticket if there is one, e.g. "Closes #12". -->

## Changes

<!-- Bullets per crate or area. -->
-

## Public API changes

<!-- Every `pub` item added, changed, or removed, with its signature.
     Say whether existing callers break. "None" if nothing public changed. -->
- None

## Downstream impact

<!-- Which other crates depend on what you changed, and whether they need
     changes now or later. For crates/common, list every crate that uses the
     touched type. "None" if nothing outside this crate is affected. -->
- None

## Architecture fit

<!-- Does this follow the crate boundaries in docs/TECH_ARCHITECTURE.md?
     Flag anything that moves a responsibility between crates, adds a
     cross-crate dependency, or needs an ADR (see docs/adr/README.md). -->
- Follows existing boundaries

## Ownership & review

<!-- Teams whose code is touched (see .github/CODEOWNERS). Flag any of:
     changes to crates/common, new or changed `unsafe`, new capabilities
     (Security & Storage review), or cross-team changes. -->
Teams touched:

## Dependencies

<!-- Each new or upgraded crate: name + version, which crate pulls it in, and
     why it is needed (what it provides or which alternative was rejected).
     Note license / maintenance / unsafe concerns. Write "None" if unchanged. -->
- None

## How to verify

<!-- Steps a reviewer can run: the command, the input (fixture under tests/ or
     an inline snippet), and what they should see. Attach a screenshot for
     anything visible. -->
1.

## Testing

<!-- Run these locally before pushing; they are the same checks CI runs.
     Tick a box only if the command ran and passed on this branch. -->
- [ ] `make lint`
- [ ] `make test`

<!-- Tests added and what each covers; then what is intentionally NOT
     covered and why. A deliberately partial feature needs a regression test
     for the supported part (AGENTS.md); name it. -->
Tests added:
-

Not covered:
-

## Questions for reviewers

<!-- Anything you were unsure about: naming, which crate something belongs
     in, whether an approach is acceptable. Asking is encouraged. "None" is
     fine. -->
- None

## Limitations / follow-ups

<!-- Intentionally partial features, TODOs, or "None". -->

<!-- If an AI agent helped open this PR, end with a line such as:
     _Opened with the help of Claude Code._ -->
