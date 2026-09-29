---
name: fix-ci
description: Find failing PR checks, inspect logs or external check links, and apply focused fixes
---

# Fix CI

## Trigger

Branch or PR CI is failing and needs a fast, iterative path to green checks.

## Tooling

Use whatever GitHub access your agent has. The steps are the same either way:

- **`gh` CLI** (typical for Codex and local terminals), if installed and
  authenticated.
- **GitHub MCP / harness tools** (typical for Claude Code on the web and other
  hosted agents), for example `pull_request_read`, `actions_list`,
  `get_check_run`, and `get_job_logs`.

## Workflow

1. Resolve the active PR for the current branch (or the branch/PR the user
   named) and list its checks with name, state, and link.
   - `gh`: `gh pr checks --json name,bucket,state,workflow,link`
   - MCP: read the PR's head SHA, then list its check runs / workflow runs.
2. Inspect failed jobs and extract the first actionable error. Use GitHub
   Actions logs when available (`gh run view --log-failed`, or `get_job_logs`);
   otherwise use the check link to identify the failing command or service.
3. Reproduce locally with the same command CI runs (`make lint`, `make test`),
   then apply the smallest safe fix and confirm it passes locally.
4. Push, re-check the PR check set, and repeat until green.

## Guardrails

- Fix one actionable failure at a time.
- Prefer minimal, low-risk changes before broader refactors.
- Do not skip or disable tests to get green.
- Treat the PR's check list as the source of truth for overall CI state.

## Output

- Primary failing job and root error
- Fixes applied in iteration order
- Current CI status and next action
