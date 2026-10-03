# Architecture docs: start here

This directory is the technical source of truth for how the browser is put
together. It is written for two readers at once: a student about to change a
crate, and the AI agent helping them. Both should read **this file first**,
then load only the documents the table below points to. Do not read the whole
directory for every task; that is what this router exists to prevent.

`docs/TECH_ARCHITECTURE.md` remains the project charter (teams, process,
workflow, review rules). This directory holds the technical detail the charter
defers to.

## How to use this directory

1. Find your task in the table below. Read the listed documents, in order.
2. The crate document for every crate you touch is always required.
3. If a crate document's **Current state** section disagrees with the code,
   the code has moved; fix the document in the same PR.
4. If your change would violate an **Invariant** below or a **Contract**
   document, stop and raise it: the fix is an ADR or a Scrum of Scrums
   decision, not a quiet workaround.
5. A human who knows the docs may hand an agent a shorter list. The agent
   should still skim this table and say if it believes another document
   applies.

## Which documents to read for which task

| Task | Read, in order |
| --- | --- |
| Any change to a single crate's internals | [`overview.md`](overview.md) §Invariants, `crates/<crate>.md`, [`conventions.md`](conventions.md) |
| Adding or changing a `pub` item | the above, plus every `contracts/*.md` that lists the crate |
| HTML parsing, DOM structure, DOM queries | [`contracts/dom.md`](contracts/dom.md), [`crates/html.md`](crates/html.md), [`crates/common.md`](crates/common.md) |
| CSS parsing, cascade, computed style | [`contracts/style-pipeline.md`](contracts/style-pipeline.md), [`crates/css.md`](crates/css.md), [`contracts/dom.md`](contracts/dom.md) |
| Layout, box tree, text flow | [`contracts/style-pipeline.md`](contracts/style-pipeline.md), [`contracts/paint-render.md`](contracts/paint-render.md), [`crates/layout.md`](crates/layout.md) |
| Painting, display lists, rasterization, compositing | [`contracts/paint-render.md`](contracts/paint-render.md), [`crates/paint.md`](crates/paint.md), [`crates/render.md`](crates/render.md) |
| JavaScript lexer, parser, interpreter, values | [`contracts/scripting.md`](contracts/scripting.md), [`crates/js.md`](crates/js.md) |
| Web APIs (`document`, `fetch`, timers, `localStorage`, console) | [`contracts/scripting.md`](contracts/scripting.md), [`crates/webapis.md`](crates/webapis.md), then the contract for whatever the API reaches: [`contracts/dom.md`](contracts/dom.md), [`contracts/networking.md`](contracts/networking.md) |
| HTTP, caching, cookies on the wire, CORS | [`contracts/networking.md`](contracts/networking.md), [`crates/net.md`](crates/net.md), [`crates/security.md`](crates/security.md) |
| Origins, same-origin policy, cookie policy, permissions | [`contracts/networking.md`](contracts/networking.md), [`crates/security.md`](crates/security.md) |
| `localStorage`, IndexedDB, persistence | [`contracts/networking.md`](contracts/networking.md) §Storage, [`crates/storage.md`](crates/storage.md), [`crates/security.md`](crates/security.md) |
| Tabs, navigation, address bar, browser UI | [`contracts/ui.md`](contracts/ui.md), [`crates/chrome.md`](crates/chrome.md), [`crates/browser.md`](crates/browser.md) |
| Native windows, platform shells | [`contracts/ui.md`](contracts/ui.md), [`crates/platforms.md`](crates/platforms.md), [`crates/browser.md`](crates/browser.md) |
| DevTools, console, inspector | [`contracts/ui.md`](contracts/ui.md) §DevTools, [`crates/devtools.md`](crates/devtools.md), [`contracts/scripting.md`](contracts/scripting.md) §Console |
| Wiring crates together, startup, the `browser` binary | [`overview.md`](overview.md), [`crates/browser.md`](crates/browser.md), [`contracts/ui.md`](contracts/ui.md) |
| Changing `crates/common` | [`crates/common.md`](crates/common.md), [`overview.md`](overview.md) §Shared types, and every crate doc that lists the touched type |
| Adding a third-party dependency | [`conventions.md`](conventions.md) §Dependencies |
| Reviewing a PR for architecture fit (Scrum of Scrums) | `crates/<crate>.md` for each crate touched, [`overview.md`](overview.md) §Dependency rules, [`gap-report-2026-10.md`](gap-report-2026-10.md) |
| Deciding whether something needs an ADR | [`overview.md`](overview.md) §Decisions that need an ADR, [`../adr/README.md`](../adr/README.md) |
| Planning a team's next sprint | [`gap-report-2026-10.md`](gap-report-2026-10.md) filtered by your team, then your crate docs |

## Invariants every change must respect

These are short on purpose. The reasoning is in [`overview.md`](overview.md).

1. **Dependencies flow one way**, down the layering in `overview.md`. A crate
   never depends on a crate above it. `js` never depends on `webapis`; engine
   crates never depend on `chrome`, `devtools`, or `browser`.
2. **One DOM, one `NodeId`.** The DOM arena lives in `html`; every crate
   addresses nodes with `common::ids::NodeId`. No casts between id types.
3. **Each boundary passes plain data**, not callbacks into another crate's
   internals. A type that crosses a boundary is owned by the crate that
   produces it and documented in the matching `contracts/*.md`.
4. **No `todo!()` reachable from a public function on a non-empty input**
   unless the matching contract test is `#[ignore]`d with a `TODO(crate)`
   reason. Prefer returning `common::error::BrowserError::Unimplemented`.
5. **Policy lives in `security`**, never re-implemented in `net`, `storage`,
   or `webapis`. Those crates call `security`; `security` calls none of them.
6. **Every `pub` item has a doc comment** stating purpose, units, and
   ownership (see `AGENTS.md`). `#![forbid(unsafe_code)]` stays in every
   crate unless an ADR says otherwise.
7. **Placeholders are labelled.** Scaffolding from Scrum of Scrums carries a
   `NOT AUTHORITATIVE` comment; the owning team may reshape it freely and must
   remove the label when they do.

## Directory map

| Path | What it holds | Maintained by |
| --- | --- | --- |
| `README.md` | this router | Scrum of Scrums |
| `overview.md` | the target architecture: pipeline, layering, dependency rules, process model, shared-type policy | Scrum of Scrums + all teams |
| `conventions.md` | Rust conventions: errors, docs, tests, placeholders, dependencies | Scrum of Scrums + all teams |
| `contracts/` | one document per cross-team boundary; the signatures teams agree on | the teams on both sides |
| `crates/` | one document per crate: purpose, allowed dependencies, intended public contract, current state, gaps | the owning team |
| `gap-report-2026-10.md` | the audit of the repo against this architecture, dated | Scrum of Scrums |
| `../adr/` | decisions and their reasons | all teams |

## Keeping these documents honest

- A crate document is maintained by the crate's team. Change it in the same
  PR as the code it describes. Review follows `.github/CODEOWNERS`: this whole
  directory is listed for every team, so Scrum of Scrums will ask the owning
  team to look at crate-doc changes even when GitHub does not require it.
- Contract documents change only with agreement from both sides; ask for a
  reviewer from each team on the PR.
- The gap report is a snapshot. When a gap closes, mark it closed in place
  with the PR number rather than deleting the row, so the history stays
  readable. A new audit gets a new dated file.
- If a document is wrong and you cannot fix it in your PR, open an issue
  titled `docs(architecture): ...` and link it from the document.
