# Crate: `html`

**Purpose:** turn HTML text into the DOM arena, and own the DOM's read and
mutation API that every other team uses.
**Maintained by:** HTML Team.
**Contracts:** [`../contracts/dom.md`](../contracts/dom.md) (primary),
[`../contracts/style-pipeline.md`](../contracts/style-pipeline.md) Stage 2.

## Dependencies

- May depend on: `common`.
- Depended on by: `css`, `layout`, `webapis`, `devtools`, `browser`.
- Must never depend on: `css`, `layout`, `js`, anything above Layer 1.

## Intended public contract

See `contracts/dom.md` for the full shape. Summary of what `html` exports:

- `Document` (the arena), `Node`, `NodeKind`, `ElementData`, `Attribute`,
  `Namespace`, `SourceSpan`, `QuirksMode`.
- `parse_document(&str) -> Document`: a total parser following the WHATWG
  tokenizer/tree-construction model closely enough to handle real pages
  (implied `<html>/<head>/<body>`, void elements, misnested formatting,
  raw-text elements like `<script>`/`<style>`, character references,
  comments, doctype → quirks mode).
- Read API on `Document`: `root`, `node`, `children`, `parent`, `element`,
  `attribute`, `text_content`, `descendants`, `get_element_by_id`.
- Mutation API on `Document`: `create_element`, `create_text`,
  `append_child`, `insert_before`, `remove_child`, `set_attribute`,
  `remove_attribute`, `set_text`, plus `take_mutations()` for invalidation.
- Node identity is `common::ids::NodeId`. No crate-local id type.
- `html` exports exactly one DOM model. Tokens are an internal detail unless
  DevTools asks for them.

Selector matching: see `contracts/dom.md` §Selectors. The existing
`query_selector` stays until the HTML and CSS teams decide where the matcher
lives; do not add a second matcher.

## Current state (2026-10-03)

About 1,100 lines; the most complete engine crate.

- **Arena parser exists:** `parse_raw_html(String) -> HtmlDocument` builds
  `HtmlDocument { nodes: Vec<Node>, root, quirks_mode }` with open-element
  stack, void elements, SVG/MathML namespaces, comments, doctype, spans.
  Does not compute `QuirksMode`, does not insert implied `<html>/<head>/<body>`,
  does not decode character references, does not treat `<script>`/`<style>`
  as raw text.
- **Id type:** `html::NodeId(pub usize)`, not `common::ids::NodeId` (G-01).
  `webapis` and `layout` cast with `as`.
- **Three DOM models coexist** (G-02): the arena; the older flat
  `Dom { Vec<LegacyNode> }` + `HTMLElement` + `tokenize`/`parse` keyed by
  `common::ids::NodeId`; and `src/simple_html_parser.rs`, a third parser
  not declared in `lib.rs` (dead, does not compile, has a doctype slice bug).
- **Mutation:** only `push_node` and `append_child` (which panics on a bad
  id and does not unlink a previous parent). No removal, no attribute
  mutation, no mutation log (G-03).
- **Queries:** `get_element_by_id`, `query_selector` (compound selectors,
  comma lists, `SelectorError`), and a `Query` trait with a
  `#[allow(non_snake_case)] fn Query(...)` that scans detached nodes too
  (G-32).
- `Location` is unused; `HTMLDocument` is an alias for `HtmlDocument`.
- Three copies of the void-element list (parser, tokenizer, dead parser).
- No `#![warn(missing_docs)]`; most `pub` items lack doc comments.

## Gaps owned by this crate

G-01, G-02, G-03, G-32, and the doc-comment debt. Suggested order: adopt
`common::ids::NodeId` and delete `html::NodeId` (one PR, touches `layout`
and `webapis` call sites); delete the legacy model and dead parser; add the
mutation API with `take_mutations`; then parser conformance work.

## Tests

- `tests/contract.rs`: 19 active tests over the arena, tokenizer, legacy
  parser, `get_element_by_id`, `query_selector`, malformed input.
- Fixtures: `tests/html/simple.html`, `tests/html/valid.html` via
  `include_str!`. `crates/html/example.html` is unused; delete or move.
- Missing: implied-element insertion, character references, raw-text
  elements, quirks mode, mutation API.

## Read next

`contracts/dom.md`, then `crates/css.md` and `crates/webapis.md` to see what
your consumers need from the DOM.
