# CSS Parser and Style Resolution Plan

## Purpose and first vertical slice

The CSS crate owns CSS syntax, selector matching, the cascade, and computed
values. Its first useful result is a style map that layout can consume without
reparsing CSS source:

```text
HTMLDocument + stylesheet sources
    -> CSS tokens and diagnostics
    -> rules, selectors, declarations, and source order
    -> matched declarations and cascade winners
    -> typed computed styles for each HTML element
    -> layout::StyledDom / layout input
```

The implementation should provide a small, deterministic, recoverable subset.
Unsupported syntax is ignored or rejected in a documented way; it must never
panic or accidentally style a different element. Correctness and clear
ownership are more important than implementing the whole CSS specification.

## Current repository state

`crates/css/src/lib.rs` is currently a single-file scaffold:

- `Selector(String)` and `specificity` operate on source text;
- `Declaration`, `Rule`, and `Stylesheet` contain strings only;
- `Dom` contains only shared `NodeId`s, not the HTML data needed for matching;
- `ComputedStyles` is a placeholder containing declarations, not computed
  values;
- `parse_stylesheet` splits on braces and lines, so strings, functions, and
  nested delimiters are unsafe;
- `style_document` still reaches `todo!` for any non-empty document or sheet;
- `crates/css/tests/contract.rs` has two ignored scaffold tests.

The HTML crate already has a public arena with element names, attributes,
parent links, and child order (`html::HTMLDocument`, `html::Node`, and
`html::ElementData`). The layout crate currently builds `StyledDom` from HTML
node IDs and has no style field. The plan must therefore include an explicit
CSS-facing DOM view and a layout handoff; neither can remain an implied future
step.

The current interface and implementations are disposable scaffolding, not the
target API. `Selector`, `Specificity`, `Declaration`, `Rule`, `Stylesheet`,
`Dom`, `ComputedStyles`, `parse_stylesheet`, `specificity`, and
`style_document` must be replaced by the new syntax, matching, cascade, and
computed-style model. This is a replacement/migration, not a request to keep
the current string-based behavior and progressively rewrite its internals.

## Scope

### Included in the first complete slice

- comments, CSS whitespace, escapes, UTF-8 input, and source spans;
- qualified rules with comma-separated selectors;
- type, universal, ID, class, descendant, and child selectors;
- attribute presence and exact-value selectors, with documented limitations;
- declarations containing strings, functions, parentheses, brackets, and
  semicolons inside nested values;
- `id`, `class`, and `style` attributes from the HTML DOM;
- deterministic matching, specificity, importance, and source order;
- typed values for the properties currently needed by layout;
- initial values, inheritance, CSS-wide keywords, and simple shorthand
  expansion for the supported property set;
- recoverable diagnostics, bounded work, and no parser panics;
- a style result that layout can consume directly.

### Explicitly deferred

Media and container queries, nesting, animations and transitions, CSSOM
mutation, vendor quirks, full custom-property substitution, complex selectors
(`:is`, `:where`, `:not`, pseudo-elements), all at-rules, and complete CSS
property coverage are later milestones. Their absence must be represented by
diagnostics or a documented unsupported result, not by approximating their
semantics.

## Non-negotiable design contracts

### Ownership and crate boundaries

The CSS crate owns parsed CSS and computed-style semantics. HTML owns the DOM
arena and mutation rules. Layout owns layout-specific structures. The preferred
boundary is a read-only CSS trait/view implemented by an HTML adapter, rather
than cloning the DOM or making CSS depend on private HTML internals. If the
architecture instead moves shared DOM data into `common`, record that decision
in an ADR before changing the public contract.

The existing architecture document names
`css::style_document(dom: &Dom, stylesheets: &[Stylesheet]) -> ComputedStyles`
as the interface, but its placeholder `Dom` is insufficient. Replace it with a
real, documented view that can provide:

- element name and namespace;
- ID, class, inline-style, and arbitrary attribute lookup;
- parent and ancestor traversal;
- previous-sibling / child-order information for future combinators;
- stable document order and detached-node behavior.

### Stable public model

Use owned syntax types rather than strings as the semantic representation. The
names may change during implementation, but the following information must be
available. These types replace the current public structs; they are not fields
to add onto the existing string-only structs:

```rust
pub struct Stylesheet {
    pub rules: Vec<StyleRule>,
    pub diagnostics: Vec<ParseDiagnostic>,
}

pub struct StyleRule {
    pub selectors: Vec<ComplexSelector>,
    pub declarations: Vec<Declaration>,
    pub source_order: usize,
    pub span: SourceSpan,
}

pub struct Declaration {
    pub property: PropertyName,
    pub value: ComponentValue,
    pub important: bool,
    pub span: SourceSpan,
}
```

Every public type, field, method, enum variant, and constant needs Rust
documentation describing purpose, units, ownership, defaults, and intentional
limitations as applicable. Keep compatibility wrappers only while callers are
migrated. A wrapper is acceptable only as a temporary forwarding boundary with
an explicit removal task; the scaffold types must not become a second semantic
model or remain the long-term public interface.

### Parsing and recovery

Parsing returns a usable partial result plus diagnostics. Diagnostics have a
severity, stable byte span, and actionable message. Recovery must advance to a
safe semicolon, balanced block boundary, or EOF. Unterminated comments,
strings, functions, and blocks are diagnosed and terminate only the affected
construct. Add input-size, diagnostic-count, nesting-depth, selector-depth,
and token/value-length limits so hostile CSS cannot cause unbounded work.

### Cascade and computed values

Represent cascade precedence explicitly: declaration importance, origin/source
category, inline style status, specificity, and source order. A stylesheet's
order and a rule's selector-list member must be preserved. Each matched
selector contributes its own specificity; a rule does not use the maximum
specificity of an unrelated selector in the same list.

The computed-style layer owns property metadata: inheritance, initial value,
CSS-wide keywords (`initial`, `inherit`, `unset`), shorthand expansion, and
typed value parsing. Values requiring context (percentages, relative lengths,
font sizes, and viewport units if added) must receive an explicit computation
context; do not hide unresolved source strings in layout-facing output.

## Implementation phases

Each phase leaves the crate compiling and adds at least one active regression
test. A phase is complete only when its output and verification gate below are
true; the phases are ordered by dependency, not by file size.

### Phase 0 — Freeze the contracts and fixtures

**Purpose:** remove ambiguity before the parser creates cross-crate APIs.

**Work:**

1. Decide whether CSS consumes an HTML adapter trait or shared `common` DOM
   data. Document ownership, lifetimes, node identity, detached nodes, and
   mutation/invalidation expectations in an ADR or architecture update.
2. Define the layout-facing computed-style shape and the minimum property set:
   `display`, `color`, `background-color`, `width`, `height`, `margin`,
   `padding`, `font-size`, and text alignment as needed by current layout.
3. Define stylesheet source metadata: author/UA origin, stylesheet order,
   inline-style precedence, and whether disabled or unsupported sources are
   supplied by the caller. CSS should receive already-selected stylesheet
   text; fetching and `<link>` loading belong elsewhere.
4. Add representative HTML/CSS fixtures for matching, cascade, malformed
   input, inline styles, inheritance, and layout handoff.

**Exit criteria:** the CSS/HTML and CSS/layout signatures are written down;
tests can construct a real HTML document and identify its elements by stable
IDs; the replacement boundary is recorded; no phase depends on the placeholder
`Dom` shape continuing unchanged.

### Phase 1 — Tokenization, spans, diagnostics, and limits

**Purpose:** provide one safe lexical foundation for every later parser.

Create focused modules such as `token.rs`, `tokenizer.rs`, and
`diagnostics.rs`. Recognize identifiers and escapes, hashes, numbers,
dimensions, percentages, strings, comments, whitespace, punctuation, function
tokens, and delimiters. Preserve source spans and enough component-value data to
debug and report errors, but do not perform property-specific validation here.

Test comments between tokens, escaped quotes, CRLF, null bytes, non-ASCII
input, EOF in every unfinished token, nesting limits, and deterministic output.

**Exit criteria:** tokenizer tests pass; all malformed-token paths produce a
diagnostic or a safe token; no tokenizer path panics or loops without
advancing.

### Phase 2 — Token-driven stylesheet and declaration parsing

**Purpose:** replace delimiter splitting with balanced, recoverable syntax.

Create `parser.rs` for qualified rules, declaration blocks, component values,
and the explicit at-rule policy. Parse comma-separated prelude text without
losing spans, require a balanced block, preserve declaration order and
duplicates, validate property-name syntax, and recognize `!important` only as
the final declaration suffix after comments/whitespace are ignored.

For the first slice, skip unsupported at-rule blocks as complete balanced
constructs and emit `UnsupportedAtRule`; never parse their nested contents as
top-level style rules. Accept empty declarations and a final declaration
without a semicolon. Recover a malformed declaration at the next safe
semicolon or block end, and a malformed rule at the next balanced block.

**Exit criteria:** strings/functions containing `;` or `}` parse correctly;
malformed declarations do not hide later valid declarations; multiple rules
remain independent; the parser returns diagnostics instead of panicking.

### Phase 3 — Selector AST, validation, matching, and specificity

**Purpose:** make selector behavior structural and testable independently of
the cascade.

Create `selector.rs` with `ComplexSelector`, compound selectors, simple
selectors, and combinators. Initially support type, universal, ID, class,
attribute presence/equality, descendant, and child selectors. Reject empty
members, malformed escapes, unsupported pseudo-classes, and pseudo-elements
with precise diagnostics. Keep adjacent/general sibling parsing deferred until
the DOM view exposes the required semantics.

Implement right-to-left matching over the read-only DOM view. Normalize HTML
element and attribute names as required, preserve case-sensitive attribute
values where applicable, handle missing attributes and detached nodes safely,
and preserve document order without duplicate matches.

Compute specificity from the AST as `(ids, classes/attributes, types)`.
Document the intentionally deferred rules for selector functions, nesting,
scoped styles, and inline styles.

**Exit criteria:** AST, matching, and specificity tests cover selector lists,
combinators, escapes, case rules, and invalid selectors; `#main > .card`
computes `(1, 1, 0)` and `div.card[data-kind="x"]` computes `(0, 2, 1)`.

### Phase 4 — HTML adapter and source collection

**Purpose:** connect the independent parser to the repository's actual DOM and
make every cascade input explicit.

Implement the agreed adapter over `html::HTMLDocument` (or the agreed shared
view). Add read-only access to names, namespace, attributes, parent/ancestor
links, sibling order, and stable IDs. Extract inline `style` declarations
through the same declaration parser, with their source span and inline origin.

Keep stylesheet loading, URL resolution, and network behavior outside CSS.
The caller supplies stylesheet sources in order with origin metadata. Define
what happens for duplicate IDs, empty class tokens, invalid attribute values,
detached nodes, and non-HTML namespaces.

**Exit criteria:** real HTML trees match type/class/ID/descendant/child
selectors; inline declarations participate in the input model; no full DOM
clone is created; tests cover missing attributes, duplicate IDs, document
order, and detached nodes.

### Phase 5 — Cascade engine and property/value model

**Purpose:** turn matches into one typed declaration per supported property.

Create `cascade.rs` and `value.rs`. For each element, collect declarations
from every matching rule and inline style, then compare candidates by explicit
importance, origin, inline status, specificity, and source order. Preserve
duplicates until this comparison is complete.

Add property metadata for the initial layout set, including initial values,
inheritance, CSS-wide keywords, shorthand expansion, accepted units, and
invalid-value behavior. Parse colors, display values, lengths, percentages,
font sizes, and box sides into typed values. Pass a computation context for
viewport dimensions, parent style, and font information where required.

Unknown properties, unsupported values, and unsupported `!important` cases
are ignored with diagnostics. Custom properties remain preserved only if the
chosen API explicitly marks them as deferred; they must not be silently
treated as ordinary properties.

**Exit criteria:** tests prove ID/class/type precedence, equal-specificity
source order, importance, inline style precedence, inheritance, initial values,
CSS-wide keywords, shorthand expansion, and invalid-value recovery.

### Phase 6 — Computed-style output and layout handoff

**Purpose:** make the first vertical slice usable by layout.

Replace the placeholder `ComputedStyles` with a documented node-to-computed-
style map whose values contain no raw CSS that layout must reinterpret. This is
a new result type and contract, not a rewrite of `Vec<(NodeId, Vec<Declaration>)>`.
Add a
conversion or constructor for `layout::StyledDom` that preserves HTML node IDs
and attaches the computed style needed by box construction. Keep CSS values and
layout geometry distinct: CSS computes values; layout resolves geometry that
depends on containing blocks when the contract requires it.

Implement `style_document` for non-empty documents and remove the normal-path
`todo!`. Define empty documents, invalid node IDs, missing stylesheets, and
unsupported properties as safe, deterministic cases.

**Exit criteria:** a small parsed HTML document and stylesheet produce computed
`display`, color, and dimensions; layout consumes them without reparsing CSS;
the CSS-to-layout integration test verifies the expected style on the expected
node.

### Phase 7 — Public API cleanup, invalidation, and hardening

**Purpose:** finish the crate as a maintainable subsystem rather than a demo.

Introduce the replacement parsing and styling entry points, migrate all call
sites and tests, then remove the old public functions and types. If a temporary
`parse_stylesheet` compatibility function is retained, it must forward to the
new parser, be marked for removal, and have no independent behavior. Do not
preserve string-based `Selector`, `Rule`, `Declaration`, placeholder `Dom`, or
declaration-backed `ComputedStyles` as alternate production paths. Document
cache keys and style invalidation boundaries for future DOM and stylesheet
mutations; the first slice may recompute the whole document, but must not imply
that stale styles are valid after mutation.

Add fuzz/property tests for arbitrary UTF-8 CSS and security tests for input
and nesting limits. Confirm repeated parsing and matching are deterministic.

**Exit criteria:** public APIs are documented, no normal non-empty path reaches
`todo!` or `unimplemented!`, unsupported input fails closed, and the two
ignored contract tests are active or replaced by equivalent stronger tests.

## Test strategy

### Unit tests

Cover token boundaries, comments, escapes, spans, unfinished input, balanced
blocks, component values, declaration recovery, selector ASTs, matching,
specificity, `!important`, property parsing, shorthand expansion, inheritance,
initial values, and CSS-wide keywords.

### Contract and integration tests

Update `crates/css/tests/contract.rs` for one rule, multiple rules, selector
lists, precedence, source order, malformed recovery, and unsupported syntax.
Add a CSS/HTML test using `html::parse_raw_html`, then add a CSS/layout test
that verifies `StyledDom` receives typed computed styles. Keep parser tests
independent of layout and keep rendering out of this first integration test.

### Fuzz and property tests

For arbitrary UTF-8 CSS, assert that parsing never panics, diagnostics have
valid spans, recovery advances or terminates, matching never indexes outside
the DOM, repeated parsing is deterministic, and unsupported input cannot
invent an arbitrary computed property.

## Delivery and verification gates

Suggested delivery order:

1. Contracts and fixtures.
2. Tokens, spans, diagnostics, and limits.
3. Stylesheet/declaration parser.
4. Selector AST, matching, and specificity.
5. HTML adapter, inline styles, and source metadata.
6. Cascade and typed computed values.
7. Replace the scaffold API, complete the layout handoff, and remove old
   implementations.
8. Invalidation notes, fuzzing, and documentation cleanup.

For every phase run:

```text
cargo fmt --all
cargo test -p css
cargo clippy -p css --all-targets -- -D warnings
```

For phases that change `common`, `html`, or `layout` contracts also run:

```text
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Before completion, verify that all new public APIs have Rust documentation,
the ignored parser/specificity backlog is gone, malformed CSS is recoverable,
computed styles are consumable by layout, and the public behavior matches the
documented unsupported-syntax and resource-limit policies.

## Risks and coordination decisions

- **DOM boundary:** the current `common::NodeId` is only an arena handle, and
  `html::HTMLDocument` owns the actual node data. Agree on the adapter/shared
  view before selector matching is treated as complete.
- **Layout contract drift:** `layout::StyledDom` currently contains only IDs.
  The computed-style attachment must be coordinated with layout and documented
  as a cross-crate API change.
- **Cascade incompleteness:** stylesheet order, origin, inline styles,
  inheritance, CSS-wide keywords, and context-dependent units must be defined
  together; otherwise a parser can appear correct while producing wrong styles.
- **Recovery and resource use:** balanced token parsing plus explicit limits is
  required before accepting complex values or untrusted page CSS.
- **API replacement:** the scaffold API and implementations are intentionally
  temporary. Migrate callers through a short-lived compatibility boundary if
  needed, then delete the old string-based types and code rather than
  maintaining parallel string and AST models.
- **Specification scope:** every unsupported selector, at-rule, property, and
  value needs a deliberate fail-closed behavior and a focused regression test.
