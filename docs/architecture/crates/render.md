# Crate: `render`

**Purpose:** rasterise a `DisplayList` into a `Frame` of RGBA8 pixels at a
device scale factor. CPU today; the only engine crate allowed to grow a GPU
backend.
**Maintained by:** Layout & Rendering Team.
**Contract:** [`../contracts/paint-render.md`](../contracts/paint-render.md).

## Dependencies

- May depend on: `common`, `paint`; later a GPU crate per ADR 0006.
- Depended on by: `browser`, `platforms/*` (for `Frame`).
- Must never depend on: `html`, `css`, `layout` (everything comes through
  `paint::DisplayList`).

## Intended public contract

```rust
pub struct Compositor { /* width, height in device px; scale_factor */ }
impl Compositor {
    pub fn new(width: u32, height: u32, scale_factor: f32) -> Self;
    pub fn compose(&self, list: &paint::DisplayList) -> Frame;
    pub const CLEAR_COLOR: [u8; 4];
}
pub struct Frame { pub width: u32, pub height: u32, pub pixels: Vec<u8> }   // RGBA8 row-major
```

`render` consumes `paint::DisplayItem` directly and converts CSS px to
device px with the scale factor. It defines no second item type. Text uses a
bitmap font until a font decision is made; glyph coverage is a documented
limitation, not a bug.

## Current state (2026-10-03)

About 230 lines.

- `Compositor::new(width, height)` with no scale factor; `compose(&[render::DisplayItem])`
  rasterises render's **own** `DisplayItem` (u32 device px); `compose_paint(&paint::DisplayList)`
  converts paint items with `as u32` casts, clamped at 0. (G-07, G-08)
- Software rasteriser: `FillRect` and `Text` via a 5×7 bitmap font covering
  about twelve glyphs (`A B D E H L O R S W ! , space`); other characters
  are skipped.
- Crate doc says "GPU rendering (wgpu)"; there is no GPU code (G-28).
- Has `#![warn(missing_docs)]`; documented.

## Gaps owned by this crate

G-07 (remove `render::DisplayItem`, take `paint::DisplayList` directly),
G-08 (scale factor), G-28 (fix the crate doc), G-25 (one ignored contract
test that would pass).

## Tests

- 1 unit test (empty list clears the frame).
- `tests/contract.rs`: 1 ignored test for `FillRect` that would now pass.
- Pipeline coverage via `crates/layout/tests/simple_element_pipeline.rs`.

## Read next

`contracts/paint-render.md`, `crates/paint.md`, `contracts/ui.md`
§Platform shells for how `Frame` is consumed.
