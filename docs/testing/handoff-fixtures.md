# Handoff: Unit and Integration Test Fixtures

## Goal

Give every team test files they can use **without waiting on another team**,
plus a small set of integration fixtures for the main joins between teams.
These live in this repo, separate from the live ladder site
(`handoff-ladder-site.md`).

## Three test tiers

1. **Unit fixtures:** one team's input → that team's output. No other team needed.
2. **Integration fixtures:** one join between two teams.
3. **Live ladder:** the whole browser (separate handoff).

## Expected output: two forms

- **Text dump** (the source of truth): DOM tree, parsed rules, computed styles,
  layout boxes, display list.
- **Reference screenshot** wherever there is something visual (paint, render,
  and pipeline fixtures). Generate it from Chrome at a fixed viewport, and
  record the viewport and browser version next to it.

## Step 1: Survey (do first)

Read each crate's public API in `crates/` and `docs/TECH_ARCHITECTURE.md` §3.2.
For each team boundary, record what goes in and what comes out **today**, and
mark the parts that are only planned. Fixture formats must match real types
where they exist. Where a type doesn't exist yet, propose a dump format and flag
it for the owning team. Check with the project lead before inventing a format.

## Unit fixtures (by team)

| Team | Input | Expected output |
|------|-------|-----------------|
| HTML | `.html` files | DOM tree dump |
| CSS | `.css` files; selector tests use a small hand-built DOM (data, not HTML) | Parsed rules dump; matched-rules dump |
| Layout | Pre-built styled tree (data file) + viewport | Layout box dump |
| Paint/Render | Pre-built layout tree or display list (data file) | Display list dump + reference PNG |
| Net | Local test server or recorded responses | Response dump (status, headers, body hash) |
| JS | `.js` snippets | Printed values / errors |

Layout and paint **must not** need the HTML or CSS crates to run their fixtures.
That is the whole point.

## Integration fixtures (main joins only)

- **Parse → style:** HTML + CSS in, computed styles out (`css::style_document`)
- **Style → layout:** styled DOM in, layout boxes out (`layout::layout_tree`)
- **Layout → paint:** layout tree in, display list + PNG out
- **Net → parse:** URL (local server) in, DOM out

Keep each join to a handful of fixtures. More can be added as features land.

## Layout

- `tests/fixtures/<team>/<case>/` with `input.*`, `expected.txt`, and optional `expected.png`
- `tests/fixtures/integration/<join>/<case>/` with the same layout
- `tests/fixtures/README.md`: the format for each team and how to add a case

Move or reuse the existing `tests/html/*.html` files.

## Done when

- [ ] Survey notes committed (`docs/testing/boundaries.md`)
- [ ] At least 3 unit fixtures per team, and 2 per integration join
- [ ] Each fixture's expected output reviewed for correctness against the spec or Chrome
- [ ] README explains the format and how a team runs its fixtures
- [ ] A draft PR opened with the `open-pr` skill
