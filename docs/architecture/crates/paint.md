# Crate: `paint`

**Purpose:** flatten a `LayoutTree` into an ordered, backend-independent
`DisplayList` of drawing commands in CSS pixels.
**Maintained by:** Layout & Rendering Team.
**Contract:** [`../contracts/paint-render.md`](../contracts/paint-render.md).

## Dependencies

- May depend on: `common`, `layout`.
- Depended on by: `render`, `browser`.
- Must never depend on: `html`, `css`, any graphics library.

## Intended public contract

```rust
pub fn build_display_list(tree: &layout::LayoutTree) -> DisplayList;
pub struct DisplayList { pub items: Vec<DisplayItem> }    // back to front
pub enum DisplayItem {
    FillRect { rect: Rect, color: Color },
    Text { origin: Point, run: layout::TextRun },
    // later: Border, Image, PushClip, PopClip
}
```

Uses `layout`'s `Rect`/`Color` (later `common::geometry`). Defines no
duplicate geometry types. Paint order follows CSS: background, then children
in tree order, then text.

## Current state (2026-10-03)

About 180 lines.

- `paint_document(tree: &LayoutTree, document: &HtmlDocument) -> DisplayList`
  emits a `FillRect` in a hard-coded light blue for each top-level box and a
  `Text` item for direct text children, read from the DOM via
  `layout::html_node_id`. One level deep only. `paint` depends on `html`.
  (G-06)
- Older `build_display_list(&[PaintBox]) -> DisplayList` hits `todo!()` on
  non-empty input; `PaintBox` and `paint::Rect` duplicate layout's types
  (G-07).
- Has `#![warn(missing_docs)]`; documented.

## Gaps owned by this crate

G-06 (drop the `html` dependency once `LayoutBox` carries text and colour),
G-07 (delete `PaintBox`/`paint::Rect`, keep one `build_display_list` that
takes `&LayoutTree`).

## Tests

- 1 unit test (empty input).
- `tests/contract.rs`: 2 ignored tests that call the `todo!()` path.
- Real coverage comes only through `crates/layout/tests/simple_element_pipeline.rs`.

## Read next

`contracts/paint-render.md`, `crates/layout.md`, `crates/render.md`.
