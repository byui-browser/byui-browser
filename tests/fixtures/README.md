# Test fixtures

Data files each team can test against without waiting on another team.
The runner crate is [`tests/fixture-runner`](../fixture-runner); team
boundaries are surveyed in [`docs/testing/boundaries.md`](../../docs/testing/boundaries.md).

## Layout

```
tests/fixtures/<team>/<case>/              input.*  expected.txt  [KNOWN_FAIL]
tests/fixtures/integration/<join>/<case>/  same
```

`expected.txt` is "console output": mostly Rust `{:?}` / `{:#?}` of the real
types, so there is no separate format to maintain.

| Folder | Input | Expected output | Run |
|---|---|---|---|
| `html/` | `input.html` | DOM tree, one `{:?}` of `NodeKind` per line, indented two spaces per level | `cargo test -p fixture-runner --test html` |
| `css/parse/` | `input.css` | `{:#?}` of `css::Stylesheet` | `cargo test -p fixture-runner --test css` |
| `css/match/` | `input.dom` (hand-built DOM, no HTML parser) + `input.css` | `css::style_document` output: `NodeId(n) <tag>: [prop: value; ...]` | same |
| `js/` | `input.js` | `Ok: <value>` (`Display`) or `Err: <JsErrorCategory>` | `cargo test -p fixture-runner --test js` |
| `net/` | `input.http` (a recorded raw response, served once from 127.0.0.1) | status, `redirected`, headers, body length and text | `cargo test -p fixture-runner --test net` |
| `integration/parse-style/` | `input.html` + `input.css` | same as `css/match` | `cargo test -p fixture-runner --test integration` |
| `integration/net-parse/` | `input.http` with an HTML body | same as `html/` | same |

### `input.dom`

One node per line, two spaces of indent per level. `"text"` is a text node;
anything else is `tag name=value name="value with spaces"`.

```
html
  body
    p id=lead class=note
      "Hi"
```

### `input.http`

A raw HTTP response with normal line endings. The runner converts to CRLF and
adds `Content-Length` when missing.

## Adding a case

1. Make a folder with the input file(s).
2. `BLESS=1 cargo test -p fixture-runner` writes `expected.txt` from the current code.
3. **Check it against the spec or Chrome** before committing. For HTML,
   `python3 tests/fixture-runner/spec_dom.py input.html` prints the spec tree
   (html5lib) in the same format.
4. If the code is wrong, write the spec-correct `expected.txt` and add a
   `KNOWN_FAIL` file saying why. Known failures are reported but do not fail
   CI, and `BLESS` never overwrites them. When the code is fixed the run fails
   with "now passes": delete `KNOWN_FAIL` then.

## Not covered yet

Layout, paint and render fixtures (and the style→layout and layout→paint
joins) are postponed. `layout::StyledDom` carries no styles yet and
`paint::paint_document` still needs the `HTMLDocument`, so they can't run on
data-only inputs. See `docs/testing/boundaries.md`. Reference PNGs come with
those fixtures.
