# Contract: Layout → Paint → Render → Frame

**Team:** Layout & Rendering owns all three crates, so this contract is mostly
a note to that team's future self and to `browser`/`platforms`, which consume
`Frame`.

## Why three crates

- `layout` decides **where** things are.
- `paint` decides **what to draw**, as a flat, ordered, backend-independent
  list.
- `render` decides **how to draw it** on a particular backend (CPU today,
  GPU later) at a particular device scale.

Keeping `paint` backend-free means the display list can be diffed, cached,
serialised to DevTools, or sent across a process boundary later.

## Signatures

```rust
// paint
pub fn build_display_list(tree: &layout::LayoutTree) -> DisplayList;
pub struct DisplayList { pub items: Vec<DisplayItem> }   // back to front
pub enum DisplayItem {
    FillRect { rect: Rect, color: Color },
    Text { origin: Point, run: layout::TextRun },        // or a paint-owned copy
    // later: Border, Image, PushClip/PopClip, PushTransform/PopTransform
}

// render
pub struct Compositor { /* viewport in device px, scale factor */ }
impl Compositor {
    pub fn new(width: u32, height: u32, scale_factor: f32) -> Self;
    pub fn compose(&self, list: &paint::DisplayList) -> Frame;
}
pub struct Frame { pub width: u32, pub height: u32, pub pixels: Vec<u8> }   // RGBA8, row-major, device px
```

Rules:

- `paint` depends on `layout` only. It never depends on `html` or `css`;
  everything it needs is in `LayoutBox` (see
  [`style-pipeline.md`](style-pipeline.md) Stage 3).
- There is **one** `DisplayItem` type, owned by `paint`, in CSS px (`f32`).
  `render` converts to device px internally using its scale factor; it does
  not define a second public item type.
- `Rect`, `Point`, `Color` are shared primitives. Until `common::geometry`
  exists (ADR 0003) they are owned by `layout` and re-used by `paint`; `paint`
  does not define duplicates.
- `render` is a software rasteriser until ADR 0006 chooses a GPU path. Text
  is drawn with a bitmap font for now; a real font stack is a separate
  decision. `render` must stay the only engine crate allowed to depend on a
  graphics library.
- `Frame` is the type shells present. It is plain data so a shell can copy
  it into a `softbuffer`/`winit` surface, an AppKit `NSImage`, or a PNG for
  tests without knowing anything about `render`.

## Current state (2026-10-03)

- `paint::paint_document(tree: &LayoutTree, document: &HtmlDocument)` takes
  the DOM to find text and hard-codes colours; `paint` depends on `html`.
  An older `build_display_list(&[PaintBox])` hits `todo!()` on non-empty
  input. `paint::Rect` duplicates `layout::Rect`. (G-06, G-07)
- `render::DisplayItem` (u32 device px) duplicates `paint::DisplayItem`;
  `Compositor::compose_paint` converts between them with `as` casts and no
  scale factor. `render` is CPU-only while its crate doc says wgpu. (G-07,
  G-08)
- The one end-to-end test (`crates/layout/tests/simple_element_pipeline.rs`)
  proves the chain works for `<div>Hello, browser!</div>` and writes a PNG
  under `target/`.
