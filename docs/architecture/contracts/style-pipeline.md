# Contract: HTML → CSS → Layout

**Teams:** HTML (produces `Document`), CSS Engine (produces `ComputedStyles`),
Layout & Rendering (produces `LayoutTree`).

This is the spine of page rendering. Each stage consumes the previous stage's
output type and nothing else.

## Stage 1: stylesheets

`css` parses stylesheet text. It does not know where the text came from; the
`browser` composition layer collects it from `<style>` elements, `<link
rel=stylesheet>` responses (via `net`), and `style=""` attributes, in document
order, and hands `css` a slice.

```rust
// css
pub fn parse_stylesheet(input: &str) -> Stylesheet;           // total; recovers from errors
pub fn parse_declarations(input: &str) -> Vec<Declaration>;   // for style="" attributes
pub struct Stylesheet { pub rules: Vec<Rule>, pub origin: StylesheetOrigin }
pub enum StylesheetOrigin { UserAgent, Author }
pub struct Rule { pub selectors: Vec<Selector>, pub declarations: Vec<Declaration> }
pub struct Selector { /* parsed, not a String */ }
pub struct Specificity(pub u32, pub u32, pub u32);
pub struct Declaration { pub property: Property, pub value: Value, pub important: bool }
```

`Property` and `Value` are **typed**, not `String`/`String`. A `String` value
pushes parsing into `layout`, which is the wrong crate. Start with the
properties the first vertical slice needs (`display`, `color`,
`background-color`, `width`, `height`, `margin-*`, `padding-*`, `font-size`)
and add variants as properties land.

## Stage 2: the cascade

```rust
// css
pub fn style_document(document: &html::Document, sheets: &[Stylesheet]) -> ComputedStyles;

pub struct ComputedStyles { /* side table keyed by NodeId */ }
impl ComputedStyles {
    pub fn get(&self, id: NodeId) -> Option<&ComputedStyle>;   // None for non-element nodes
}
pub struct ComputedStyle {
    pub display: Display,
    pub color: Color,
    pub background_color: Option<Color>,
    pub width: Length, pub height: Length,
    pub margin: Edges, pub padding: Edges, pub border: Edges,
    pub font_size: f32,          // CSS px
    // grows with the property set; use accessors or #[non_exhaustive]
}
```

Rules:

- `style_document` walks `document.descendants(root)` once, matches every
  rule's selectors against each element, sorts by (origin, specificity,
  source order), applies `!important`, then resolves inheritance parent-first.
- It depends on `html` for structure only (`element`, `attribute`, `parent`).
  It never mutates the document.
- A user-agent stylesheet (`css` ships one as a `const &str`) is always the
  first sheet, so `layout` can assume every element has a `display`.
- Output is a fresh `ComputedStyles` each call for now. Incremental restyle
  via `Document::take_mutations` is a later optimisation behind the same
  signature.

## Stage 3: layout

```rust
// layout
pub fn layout_tree(document: &html::Document, styles: &css::ComputedStyles, viewport: Size) -> LayoutTree;

pub struct LayoutTree { pub root: LayoutBox }
pub struct LayoutBox {
    pub node: Option<NodeId>,         // None for anonymous boxes
    pub kind: BoxKind,                // Block, Inline, AnonymousBlock, Text
    pub rect: Rect,                   // border box, CSS px, relative to the viewport
    pub background: Option<Color>,    // copied from ComputedStyle so paint needs no DOM
    pub text: Option<TextRun>,        // for text boxes: the string, color, font size
    pub children: Vec<LayoutBox>,
}
pub struct TextRun { pub text: String, pub color: Color, pub font_size: f32 }
```

Rules:

- `layout` reads `document` for tree structure and text content, and
  `styles` for everything visual. It never parses a string.
- `layout` owns the geometry types it needs until `common::geometry` exists
  (ADR 0003); then it uses those.
- **Everything paint needs is in `LayoutBox`.** Paint must not need the
  `Document` or `ComputedStyles`. That means colours, text, and font metrics
  are resolved here.
- The box tree is an owned recursive structure today. If profiling shows a
  need, it may become an arena; the public shape (`root`, `children`) stays.

## The `StyledDom` question

The charter wrote `layout_tree(styled_dom: &StyledDom, viewport)`. We
recommend the two-argument form above instead of a `StyledDom` wrapper type:
a wrapper either copies the DOM (wasteful) or holds two borrows (the same as
two arguments). If the Layout team prefers a wrapper, it is
`pub struct StyledDom<'a> { pub document: &'a Document, pub styles: &'a ComputedStyles }`
and is defined in `layout`, not `css`.

## Who calls what

`browser` (the composition crate) is the only caller of the whole chain:

```rust
let document = html::parse_document(&source);
let sheets = collect_stylesheets(&document /*, fetched link bodies */);
let styles = css::style_document(&document, &sheets);
let tree = layout::layout_tree(&document, &styles, viewport);
```

Tests that exercise the chain end to end live in `tests/` at the repository
root once `browser` is a library; until then they may live in
`crates/layout/tests/` with `paint`/`render` as dev-dependencies.

## Current state (2026-10-03)

- `css::style_document(dom: &css::Dom, ...)` takes a `css`-local placeholder
  (`Vec<NodeId>` with no structure), does not depend on `html`, and hits
  `todo!()` for any non-empty input. Nothing consumes `ComputedStyles`.
  `Declaration` is `String`/`String`. (G-04)
- `layout::layout_tree(&StyledDom, Size)` takes `layout::StyledDom`, which is
  a `Vec<NodeId>` built directly from `html::HtmlDocument` with no style data.
  Layout depends on `html`, not `css`. Every element becomes a 100 px tall
  full-width block. (G-05)
- `paint` reads text from the `Document` because `LayoutBox` has no
  `TextRun`. (G-06)
