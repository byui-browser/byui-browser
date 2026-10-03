# ADR 0002: DOM representation

- **Status**: Proposed
- **Date**: 2026-10-03
- **Deciders**: HTML Team (owner), CSS, Layout & Rendering, JS APIs, DevTools (consumers), Scrum of Scrums

## Context

The DOM is consumed by four teams besides its owner. Today the `html` crate
holds three representations (an index arena `HtmlDocument`, a legacy flat
`Dom`, and an undeclared third parser), and the arena uses its own
`html::NodeId(usize)` while `common::ids::NodeId(u32)` is used by every
consumer, forcing `as` casts at each boundary. `common::dom` is an empty
module that the charter expected to hold "the DOM skeleton".

The charter asked for the DOM representation to be one of the first three
ADRs.

## Decision

1. The DOM is an **index arena** owned by the `html` crate: a `Document`
   holding `Vec<Node>`, with parent and ordered-children links stored as ids.
2. Node identity is `common::ids::NodeId` everywhere. `html` defines no
   id type of its own. Node 0 is the document node.
3. The DOM types (`Document`, `Node`, `NodeKind`, `ElementData`,
   `Attribute`) live in `html`, not `common`. `common::dom` is deleted.
   Consumers depend on `html` directly (the layering in
   `docs/architecture/overview.md` §2 permits this for `css`, `layout`,
   `webapis`, `devtools`).
4. Style, layout and script wrappers are **side tables keyed by `NodeId`**
   owned by `css`, `layout`, `webapis`. Nodes hold no references into other
   crates' data.
5. `html` exposes a read API and a separate **mutation API**, and records
   mutations (`take_mutations`) so `browser` can drive invalidation. Only
   `webapis` (on behalf of scripts) and `html`'s own parser mutate.
6. The legacy `Dom`/`LegacyNode`/`HTMLElement` model and the undeclared
   parser are removed.

The full contract is `docs/architecture/contracts/dom.md`.

## Consequences

- Easier: one id type, no casts; consumers can hold `NodeId`s in maps;
  `Document` is `Send` if it ever needs to be; serialising a DOM snapshot
  for DevTools is a walk over a `Vec`.
- Harder: node removal must detach rather than shift indices (freed-slot
  reuse is an `html` internal); consumers must re-validate ids across
  mutations.
- Follow-up: HTML Team lands the id change and deletions as one PR touching
  `layout` and `webapis` call sites (gap G-01, G-02); then the mutation API
  (G-03). CSS and HTML decide where selector matching lives
  (`contracts/dom.md` §Selectors).

## Alternatives considered

- **`Rc<RefCell<Node>>` tree.** Rejected: borrow-checker friction at every
  consumer, no cheap ids for side tables, cycles need `Weak`.
- **DOM types in `common`** (charter §3.1). Rejected: makes every DOM change
  a nine-team review and gives `common` behaviour. Producer-owned types with
  a one-way dependency achieve the same sharing. (Generalised in the
  forthcoming ADR 0003.)
- **Keep `html::NodeId(usize)` and convert at boundaries.** Rejected: the
  casts are already in three crates and will multiply.
