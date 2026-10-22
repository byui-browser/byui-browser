# Team boundaries (survey, 2026-10-22)

What each crate takes in and gives out **today** on `main` (7e42e36), and what
is only planned. Fixture formats in `tests/fixtures/` follow these types.
None of the crates ship a text dump today. Agreed with the project lead:
`expected.txt` is "console output" (Rust `{:?}` of the real types) rather
than a new format. Layout, paint and render fixtures are postponed until
layout takes real styles.

| Boundary | Real today | Planned / missing |
|---|---|---|
| HTML | `html::parse_raw_html(String) -> HTMLDocument` (arena of `Node { parent, children, kind, span }`; `NodeKind` = Document, Doctype, Element, Text, Comment) | Has an `innerHTML`-style serializer, but no tree dump. Fixtures print `{:?}` of each node's kind, indented. |
| CSS parse | `css::parse_stylesheet_with_errors(&str) -> (Stylesheet, Vec<ParseError>)`; `Rule { selector: Selector(String), declarations }`, `AtRule`; `parse_selector_list` gives a structured AST | Fixtures print `{:#?}` of `Stylesheet`. |
| CSS match/cascade | `css::style_document(&HTMLDocument, &[Stylesheet]) -> ComputedStyles { styles: Vec<(NodeId, Vec<Declaration>)> }` — values are raw strings, no initial values | Selector matching is private (`matches_selector`); no matched-rules API. CSS needs a real `Dom`, so "hand-built DOM" fixtures are DOM data the runner turns into an `HTMLDocument`. |
| CSS → Layout | `layout::StyledDom { nodes: Vec<NodeId> }` — **no styles at all**; built only via `StyledDom::from_html_document` | Arch §3.2 says layout takes a styled DOM; real styles are not wired. Layout input file is a node list today, with a planned style column. |
| Layout | `layout::layout_tree(&StyledDom, Size) -> LayoutTree`; every node is a 100px-tall full-width block, flat under the root | Nesting, box model, text sizing. Expected dumps reflect today's behavior and will change when layout grows. |
| Layout → Paint | `paint::paint_document(&LayoutTree, &HTMLDocument) -> DisplayList` (`FillRect`, `Text`, CSS px, fixed colors) | Paint still needs `HTMLDocument` for text, so paint fixtures can't be fully HTML-free yet. `build_display_list(&[PaintBox])` is `todo!()` for non-empty input. **Flag for paint:** take text from layout, not the DOM. |
| Paint → Render | `render::DisplayItem` (device px, `u32`, `[u8;4]`), `Compositor`, `Frame`, bitmap `glyph` font | Render fixtures start from a display list data file. |
| Net | `RequestController`, `Response { status, status_text, headers, url, redirected, body, from_cache }`, providers (file, blob, page) | No recorded-response replay; `file:` provider lets fixtures avoid sockets. |
| JS | `js::eval(&str) -> JsResult<Value>`; `Value: Display`; `JsError { category, .. }` | No `console.log` capture; expected output is the completion value or error category. |

Notes:
- `crates/layout/src/main.rs` is a stray hello-world binary.
- Reference PNGs from Chrome won't match our renderer pixel-for-pixel today;
  they are the target, not a pass/fail gate.
- `main` at 7e42e36 did not compile: the `innerHTML` serializer missed
  `NodeKind::None`. Fixed in this branch with a one-line arm.
- HTML parser gaps found by fixtures (marked `KNOWN_FAIL`): no implied
  `html`/`head`/`body`; whitespace before `<html>`/`<head>` kept; whitespace
  after `</body>` placed outside `body`.
