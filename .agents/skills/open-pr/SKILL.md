---
name: open-pr
description: Prepare, validate, and open a draft GitHub pull request for the current changes following byui-browser conventions (team branch naming, CODEOWNERS reviewers, fmt/clippy/test gates). Use when the user asks to open, create, submit, or update a PR, or asks to run the pre-PR checks (lint/tests) on their behalf.
---

# Open a pull request

## Trigger

The user explicitly asks to open, create, submit, or update a pull request.
Do not open a PR on your own initiative.

The user may also ask only for the validation part ("run the PR checks",
"make sure this will pass CI", "lint and test this for me"). In that case run
step 6 alone, report the results, and stop without committing, pushing, or
opening a PR unless asked.

## Tooling

Use whatever GitHub access your agent has. The steps are the same either way:

- **`gh` CLI** (typical for Codex and local terminals), if it is installed and
  authenticated (`gh auth status`).
- **GitHub MCP / harness tools** (typical for Claude Code on the web and other
  hosted agents), for example `create_pull_request`, `list_pull_requests`, and
  `pull_request_read`.

If neither is available, push the branch and give the user the
`https://github.com/<owner>/<repo>/compare/main...<branch>` link to open the PR
by hand.

**Work in the terminal, never through a browser or GUI.** Every step in this
skill is a shell command (`git`, `cargo`, `make`, `gh`) or a GitHub API/MCP
call. Do not open, drive, or ask for permission to control Safari, Chrome,
Finder, or any other application, and do not ask the user to grant
Accessibility, Automation, or screen-control access; this applies to the
ChatGPT desktop app and Codex as much as to any other agent. If you cannot
run the command yourself, say so and give the user the exact command to run,
not a GUI walkthrough.

## Audience

This is a class project. Keep two readers in mind:

- **The submitter** is usually a freshman, often opening their first PR. When
  you fix something for them (a clippy lint, a failing test, a missing doc
  comment), tell them in one or two sentences what was wrong and why the fix
  is right, so the check teaches them rather than silently absorbing the
  lesson. Prompt them for the *why* of the change if it is not obvious from
  the diff; do not invent a motivation.
- **The reviewers** are the integration team (`@byui-browser/byui-browser-scrum-of-scrums`:
  upper-class students and the professor). Their job is to make sure every
  team's work fits together, so the PR body must answer their questions
  directly: what other crates can now call or must now change, whether the
  change matches `docs/TECH_ARCHITECTURE.md`, and how to run it. Write those
  sections precisely; write the rest in plain language.

## Workflow

1. **Inspect the working tree.** Run `git status` and
   `git diff --stat origin/main...HEAD`. Only include changes related to the
   task. If there are unrelated edits, ask before including or dropping them.
2. **Branch.** Never commit to `main`. If on `main`, create a branch named
   `team/short-description` (for example `css/cascade-layers`), taking the team
   from the crate being changed (see the README layout or `.github/CODEOWNERS`).
   If the harness or the user already chose a branch, keep it.
3. **Scope and ownership.** List the changed paths and map them to owning teams
   with `.github/CODEOWNERS`. Flag these for the PR body:
   - `crates/common/` changed: all affected teams must review.
   - New or changed `unsafe`: two experienced reviewers, plus Miri.
   - New capability (file access, device APIs, ...): Security & Storage review.
   - Several teams' crates touched: reviewers from every owning team.
   - New crate or new top-level path: it must be added in three places
     together — the workspace `members` in `Cargo.toml`, `.github/CODEOWNERS`,
     and the path→team map in `.github/workflows/pr-notify.yml` — or it will
     have no owner and no review notifications.
   If the diff mixes unrelated work, suggest splitting it into smaller PRs.
   Also suggest splitting when the diff is large (roughly more than 400
   changed lines outside tests); the integration team reviews every team's
   work and small PRs get reviewed faster and better.
4. **Audit dependencies.** Run `git diff origin/main...HEAD -- '**/Cargo.toml'
   Cargo.lock` and note every crate that is new, removed, or has a changed
   version or feature set. For each new or upgraded dependency, record for the
   PR body:
   - the crate name and version, and which workspace crate pulls it in;
   - why it is needed (what it does that the codebase could not do already,
     or what alternative was rejected and why);
   - anything reviewers should weigh: license, maintenance status, transitive
     dependency count, `unsafe`, build scripts, or network/file access.
   Prefer not adding a dependency for something small that can be written in
   place. If a `Cargo.lock` change is only an indirect bump with no direct
   change, say so in one line.
5. **Describe the change for the integration team.** Gather these from the
   diff for the PR body; they are what reviewers need to judge whether the
   work meshes with other teams' crates:
   - **Public API changes.** Every `pub` item added, changed, or removed at the
     crate root or in a public module, with its signature. Say whether any
     change breaks existing callers.
   - **Downstream impact.** Which workspace crates depend on the changed crate
     (`cargo tree -i -p <crate> --workspace` or grep the `Cargo.toml`s) and
     whether they need changes now or later. For `common`, list every crate
     that uses the touched type.
   - **Architecture fit.** Does the change respect the crate boundaries and
     responsibilities in `docs/TECH_ARCHITECTURE.md`? If it moves a
     responsibility between crates, adds a cross-crate dependency, or decides
     something the ADR README lists as needing an ADR, say so and either
     include the doc update or flag it as a follow-up for the reviewers to
     rule on.
   - **How to verify.** Concrete steps a reviewer can run: the command
     (`cargo run -p browser`, `cargo test -p <crate> <test_name>`), the input
     (a fixture under `tests/` or an inline snippet), and the expected result.
     For anything visible (`chrome`, `layout`, `paint`, `render`), attach a
     before/after screenshot or the rendered output.
   - **Tests added and gaps.** Name the new tests and what each covers, then
     state what is intentionally *not* covered and why. Per `AGENTS.md`, a
     deliberately partial feature needs a regression test for the supported
     part; name it.
   - **Where to start.** For diffs touching more than a few files, suggest a
     reading order (usually: public types, then the logic, then tests).
   - **Questions for reviewers.** Anything the submitter was unsure about
     (naming, which crate something belongs in, whether an approach is
     acceptable). Encourage the submitter to ask; the integration team would
     rather answer a question than find the problem later.
   - **Self-review.** Read the full diff once and remove debugging leftovers
     (`println!`, `dbg!`, commented-out code, unexplained `TODO`s, stray
     `allow(...)` attributes). Ask before removing anything that might be
     intentional.
6. **Validate locally. This step is mandatory and must not be skipped.** CI
   runs `make lint` and `make test` (see `.github/workflows/ci.yml`). **You
   run these commands yourself**, in the developer's checkout, so they learn
   about needed fixes now instead of waiting on GitHub CI. Do not hand the
   developer a list of commands to run; execute them, read the output, and
   report what actually happened. If the developer says they already ran the
   checks, ask for the results or re-run them anyway rather than ticking a box
   you did not verify. If the developer asks to run them themselves, wait for
   their results before continuing past this step.
   - `cargo fmt --all`
   - `make lint` (fmt check + `cargo clippy --workspace --all-targets -- -D warnings`)
   - `make test` (the full workspace, exactly as CI runs it). `make test <crate>`
     is fine as a quick inner loop while iterating, but the full run must pass
     before pushing.
   - `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` to catch
     missing docs on public items and broken doc links, per `AGENTS.md`. Then
     confirm by eye that every new public API has Rust docs.
   - Only if the diff adds or changes `unsafe`: `cargo +nightly miri test -p
     <crate>` (install with `rustup +nightly component add miri`). If nightly
     is unavailable, say so in the PR body so a reviewer can run it.
   Do not push or open the PR until every command above has been run in this
   session and passed. If something fails, fix it when the fix is in scope,
   tell the submitter what was wrong and why the fix is correct (see
   Audience), and re-run the full set. Otherwise stop and report the exact failure; do not
   open a PR you know is red. Never leave a check unrun and defer to CI, and
   never leave the template's Testing boxes unchecked with the commands
   "to be run later". If a command cannot be run in this environment (missing
   toolchain, no network for crates), say so explicitly in the PR body and to
   the user rather than implying it passed.
7. **Commit.** Short, imperative messages (for example "Add basic CSS parser
   for body background"). Add any attribution trailers your harness requires.
8. **Push** with `git push -u origin <branch>`. Never force-push a branch other
   people have pushed to.
9. **Look for an existing PR** for this branch. If one is open, update its
   title and description instead of creating a duplicate.
10. **Write the PR body.** Fill in `.github/pull_request_template.md` section
    by section, using what you gathered in steps 3–6:
    - **Summary**: the why first, then the what, and a link to the issue or
      ticket if there is one (`Closes #N`).
    - **Dependencies**: each new or changed dependency from step 4 with its
      version and reason; "None" if the manifests did not change.
    - **Public API / downstream impact / architecture fit / how to verify /
      tests and gaps / questions for reviewers**: from step 5. Write "None"
      rather than deleting a section, so reviewers can tell it was considered.
    - **Testing**: only commands you actually ran, with their real results.
    End the body with the AI disclosure line naming your agent, for example
    `_Opened with the help of Claude Code._` or `_Opened with the help of Codex._`
11. **Open the PR as a draft** against `main`, with a title in the same style
    as the commit messages. Only open it ready for review if the user asks.
    - `gh`: `gh pr create --draft --base main --title "..." --body-file <file>`
    - MCP: `create_pull_request` with `draft: true` and `base: "main"`
    Tell the submitter that a **draft does not request reviewers or notify
    the team**: CODEOWNERS review requests and the Discord notification in
    `pr-notify.yml` both fire only when the PR is opened ready or marked
    "Ready for review". When they are ready, they (or you, if asked) should
    mark it ready with `gh pr ready <number>` or the GitHub button.
12. **Follow up.** Report the PR URL and check CI status. If CI fails, continue
    with the `fix-ci` skill; if the branch conflicts with `main`, use
    `fix-merge-conflicts`.

## Guardrails

- Never push to `main`, merge, or approve a PR.
- Never control a browser or desktop application, or request OS permissions
  to do so. Use shell commands and GitHub API/MCP tools only.
- Never report a check as passing unless you ran it and it passed.
- Never push or open a PR without having run `make lint` and `make test`
  locally in this session; local validation is not optional and CI is not a
  substitute for it.
- Never commit secrets, `target/`, or editor/OS files.
- Do not skip or disable tests to get green.
- Keep the PR focused; do not refactor unrelated code.

## Output

- PR URL, branch, and draft status
- Owning teams / reviewers needed and any flags from step 3
- New or changed dependencies and why each was added (or "None")
- Public API changes and which other crates are affected (or "None")
- Validation commands run and their results, and any fixes you made with a
  one-line explanation of each for the submitter
- Reminder that the draft must be marked ready before reviewers are notified
- Current CI state and next step
