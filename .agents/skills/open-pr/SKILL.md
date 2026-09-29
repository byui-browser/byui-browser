---
name: open-pr
description: Prepare, validate, and open a draft GitHub pull request for the current changes following byui-browser conventions (team branch naming, CODEOWNERS reviewers, fmt/clippy/test gates). Use when the user asks to open, create, submit, or update a PR.
---

# Open a pull request

## Trigger

The user explicitly asks to open, create, submit, or update a pull request.
Do not open a PR on your own initiative.

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
   If the diff mixes unrelated work, suggest splitting it into smaller PRs.
4. **Validate locally.** These match CI:
   - `cargo fmt --all`
   - `make lint` (fmt check + `cargo clippy --workspace --all-targets -- -D warnings`)
   - `make test <crate>` for each changed crate, or `make test` when `common`
     or several crates changed
   - Confirm every new public API has Rust docs, per `AGENTS.md`.
   If something fails, fix it when the fix is in scope. Otherwise stop and
   report; do not open a PR you know is red.
5. **Commit.** Short, imperative messages (for example "Add basic CSS parser
   for body background"). Add any attribution trailers your harness requires.
6. **Push** with `git push -u origin <branch>`. Never force-push a branch other
   people have pushed to.
7. **Look for an existing PR** for this branch. If one is open, update its
   title and description instead of creating a duplicate.
8. **Write the PR body.** Fill in `.github/pull_request_template.md` section by
   section. List only commands you actually ran, with their real results. End
   the body with the AI disclosure line naming your agent, for example
   `_Opened with the help of Claude Code._` or `_Opened with the help of Codex._`
9. **Open the PR as a draft** against `main`, with a title in the same style
   as the commit messages. Only open it ready for review if the user asks.
   - `gh`: `gh pr create --draft --base main --title "..." --body-file <file>`
   - MCP: `create_pull_request` with `draft: true` and `base: "main"`
10. **Follow up.** Report the PR URL and check CI status. If CI fails, continue
    with the `fix-ci` skill; if the branch conflicts with `main`, use
    `fix-merge-conflicts`.

## Guardrails

- Never push to `main`, merge, or approve a PR.
- Never report a check as passing unless you ran it and it passed.
- Never commit secrets, `target/`, or editor/OS files.
- Do not skip or disable tests to get green.
- Keep the PR focused; do not refactor unrelated code.

## Output

- PR URL, branch, and draft status
- Owning teams / reviewers needed and any flags from step 3
- Validation commands run and their results
- Current CI state and next step
