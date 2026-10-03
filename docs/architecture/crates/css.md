# Crate: `css`

**Purpose:** parse stylesheets and compute the final style of every element:
selector matching, cascade, specificity, inheritance, typed values.
**Maintained by:** CSS Engine Team.
**Contracts:** [`../contracts/style-pipeline.md`](../contracts/style-pipeline.md)
Stages 1–2, [`../contracts/dom.md`](../contracts/dom.md) §Selectors.

## Dependencies

- May depend on: `common`, `html`.
- Depended on by: `layout`, `devtools`, `browser`.
- Must never depend on: `layout`, `paint`, anything above Layer 1.

## Intended public contract

```rust
pub fn parse_stylesheet(input: &str) -> Stylesheet;          // total
pub fn parse_declarations(input: &str) -> Vec<Declaration>;  // style="" attributes
pub fn style_document(document: &html::Document, sheets: &[Stylesheet]) -> ComputedStyles;
pub fn matches(document: &html::Document, id: NodeId, selector: &Selector) -> bool;
pub const USER_AGENT_STYLESHEET: &str;

pub struct Stylesheet { pub rules: Vec<Rule>, pub origin: StylesheetOrigin }
pub struct Rule { pub selectors: Vec<Selector>, pub declarations: Vec<Declaration> }
pub struct Selector { /* parsed compound/complex selector */ }
pub struct Specificity(pub u32, pub u32, pub u32);
pub struct Declaration { pub property: Property, pub value: Value, pub important: bool }
pub enum Property { Display, Color, BackgroundColor, Width, Height, MarginTop, /* … */ }
pub enum Value { Keyword(Keyword), Length(Length), Color(Color), Percentage(f32), /* … */ }
pub struct ComputedStyles { /* NodeId → ComputedStyle */ }
pub struct ComputedStyle { pub display: Display, pub color: Color, /* … */ }
```

Key rules:

- Typed `Property`/`Value`, not strings. Unknown properties are dropped at
  parse time (that is what browsers do), with a diagnostic.
- `style_document` takes the real `html::Document`. There is no `css::Dom`.
- Inheritance is resolved here; `layout` never looks at a parent's style.
- Everything in `ComputedStyle` has a value for every element (the UA sheet
  plus initial values guarantee it).

## Current state (2026-10-03)

About 200 lines in a single `lib.rs`, all marked `NOT AUTHORITATIVE`.

- `parse_stylesheet` splits on `{ } ; :` after stripping comments and
  produces `Rule { selector: Selector(String), declarations: Vec<Declaration{property: String, value: String}> }`.
  Works for simple sheets. Its doc comment still says it only handles the
  empty stylesheet (G-25).
- `specificity(&Selector)` counts `#`, `.`, and tag tokens. Doc comment also
  stale (G-25).
- `style_document(dom: &css::Dom, ...)` takes `css::Dom { nodes: Vec<NodeId> }`,
  a placeholder with no structure, and hits `todo!()` for any non-empty input.
  `css` does not depend on `html`. Nothing consumes `ComputedStyles`. (G-04)
- `ComputedStyles` is `Vec<(NodeId, Vec<Declaration>)>` with string values.
- Two contract tests are `#[ignore]`d with reasons ("parser not implemented",
  "specificity not implemented") that are no longer true (G-25).

## Gaps owned by this crate

G-04 (the big one), G-25. Suggested order: add the `html` dependency and
change `style_document` to take `&html::HtmlDocument` (it will become
`Document`); implement matching for type/class/id selectors; replace string
`Declaration` with typed `Property`/`Value` for the first few properties;
un-ignore the contract tests; coordinate with Layout on `ComputedStyle`
fields before they build against it.

## Tests

- 5 unit tests (parsing a body rule, specificity ordering, empties).
- `tests/contract.rs`: 2 tests, both ignored, both would pass now.
- No fixtures. Add `tests/css/` fixtures as the parser grows.

## Read next

`contracts/style-pipeline.md`, then `crates/layout.md` to see what Layout
needs from `ComputedStyle`, and `crates/html.md` for the DOM read API.
