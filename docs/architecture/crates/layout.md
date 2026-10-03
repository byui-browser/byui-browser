# Crate: `layout`

**Purpose:** turn the DOM plus computed styles into a tree of positioned
boxes (block, inline, text) in CSS pixels.
**Maintained by:** Layout & Rendering Team.
**Contracts:** [`../contracts/style-pipeline.md`](../contracts/style-pipeline.md)
Stage 3, [`../contracts/paint-render.md`](../contracts/paint-render.md).

## Dependencies

- May depend on: `common`, `html`, `css`.
- Dev-dependencies for end-to-end tests: `paint`, `render`, `png` (move the
  test to `tests/` at the root once `browser` is a library).
- Depended on by: `paint`, `devtools`, `browser`.
- Must never depend on: `paint`, `render`, any windowing or GPU crate.

## Intended public contract

```rust
pub fn layout_tree(document: &html::Document, styles: &css::ComputedStyles, viewport: Size) -> LayoutTree;

pub struct LayoutTree { pub root: LayoutBox }
pub struct LayoutBox {
    pub node: Option<NodeId>, pub kind: BoxKind, pub rect: Rect,
    pub background: Option<Color>, pub text: Option<TextRun>, pub children: Vec<LayoutBox>,
}
pub enum BoxKind { Block, Inline, AnonymousBlock, Text }
pub struct TextRun { pub text: String, pub color: Color, pub font_size: f32 }
pub struct Size { pub width: f32, pub height: f32 }   // CSS px; moves to common::geometry later
pub struct Rect { pub x: f32, pub y: f32, pub width: f32, pub height: f32 }
```

Key rules:

- Input is the DOM (structure, text) plus `ComputedStyles` (everything
  visual). `layout` never parses a string and never reads attributes.
- Output is self-contained: `paint` must be able to draw from `LayoutBox`
  alone. Colours and text runs are resolved here.
- Algorithm order: block layout (normal flow, margins, padding, borders,
  `width: auto`), then inline/text with a fixed-width font metric, then
  nesting, then the rest (`position`, flex) as later milestones.

## Current state (2026-10-03)

About 190 lines.

- `layout_tree(styled_dom: &StyledDom, viewport: Size) -> LayoutTree`
  matches the charter's signature by name, but `StyledDom { nodes: Vec<NodeId> }`
  is built by `StyledDom::from_html_document(&HtmlDocument)` and carries **no
  style**. Every element becomes a full-width 100 px block, all direct
  children of the root; nesting is ignored. `layout` depends on `html` and
  not on `css`. (G-05)
- `html_node_id(NodeId) -> html::NodeId` exists only to bridge the two id
  types (G-01).
- `LayoutBox { node, rect, children }` has no colour or text, so `paint`
  reads the DOM (G-06).
- `src/basics.rs` is a dead `fn main` with `println!`, not declared in
  `lib.rs` (G-26).
- Has `#![warn(missing_docs)]`; all public items documented.

## Gaps owned by this crate

G-05, G-06 (supply `TextRun`/`background` so paint can drop `html`), G-26,
and the `html_node_id` half of G-01. Depends on CSS landing a usable
`ComputedStyle` (G-04); coordinate the field list early.

## Tests

- 2 unit tests (rect containment, empty document fills viewport).
- `tests/contract.rs`: 1 ignored test that would now pass (G-25).
- `tests/simple_element_pipeline.rs`: 2 end-to-end tests
  (`parse → layout → paint → render`), asserting pixel colours and writing
  `target/layout-test-output/simple-element.png` (G-27). This is currently
  the only HTML-to-pixels test in the repository; keep it green.

## Read next

`contracts/style-pipeline.md` Stage 3, `contracts/paint-render.md`,
`crates/css.md` §Intended contract for `ComputedStyle`.
