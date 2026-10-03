# Contract: the DOM

**Producer:** `html` (HTML Team). **Consumers:** `css`, `layout`, `webapis`,
`devtools`. **Shared identifier:** `common::ids::NodeId` (all teams).

The DOM is the one data structure almost every team touches, so its shape is
the most consequential contract in the repository. ADR 0002 (proposed) records
the decision below.

## Representation

- The DOM is an **index arena**: `html::Document` owns `Vec<Node>`; nodes
  refer to each other by `common::ids::NodeId`, which is the index.
- There is exactly **one** `NodeId` type in the workspace, `common::ids::NodeId`.
  `html` does not define its own. Consumers never cast between id types.
- Node 0 is always the `Document` node. Ids are stable for the life of the
  document; removal detaches a node rather than shifting indices (freed slots
  may be reused later behind a generation counter if needed; that is an
  internal detail of `html`).
- Each node stores `parent`, ordered `children`, its `NodeKind`, and an
  optional source span. Sibling links are derived, not stored, until a
  measured need appears.
- There is no `Rc`, no `RefCell`, and no reference from a node into another
  crate's data. Style, layout and script wrappers are **side tables keyed by
  `NodeId`**, owned by `css`, `layout` and `webapis` respectively.

## Intended public surface (owned by `html`)

```rust
pub struct Document { /* private */ }
pub struct Node { pub parent: Option<NodeId>, pub children: Vec<NodeId>, pub kind: NodeKind, pub span: Option<SourceSpan> }
pub enum NodeKind { Document, Doctype { .. }, Element(ElementData), Text(String), Comment(String) }
pub struct ElementData { pub name: String, pub namespace: Namespace, pub attributes: Vec<Attribute> }
pub struct Attribute { pub name: String, pub value: String }

// Parsing (total: never fails on malformed input)
pub fn parse_document(source: &str) -> Document;

// Read access
impl Document {
    pub fn root(&self) -> NodeId;
    pub fn node(&self, id: NodeId) -> Option<&Node>;
    pub fn children(&self, id: NodeId) -> &[NodeId];
    pub fn parent(&self, id: NodeId) -> Option<NodeId>;
    pub fn element(&self, id: NodeId) -> Option<&ElementData>;
    pub fn attribute(&self, id: NodeId, name: &str) -> Option<&str>;
    pub fn text_content(&self, id: NodeId) -> String;
    pub fn descendants(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_; // tree order
    pub fn get_element_by_id(&self, value: &str) -> Option<NodeId>;
    pub fn query_selector(&self, selector: &str) -> Result<Option<NodeId>, SelectorError>;
    pub fn query_selector_all(&self, selector: &str) -> Result<Vec<NodeId>, SelectorError>;
}

// Mutation (the only way any crate changes the tree)
impl Document {
    pub fn create_element(&mut self, name: &str) -> NodeId;
    pub fn create_text(&mut self, text: &str) -> NodeId;
    pub fn append_child(&mut self, parent: NodeId, child: NodeId) -> Result<(), DomError>;
    pub fn insert_before(&mut self, parent: NodeId, child: NodeId, before: NodeId) -> Result<(), DomError>;
    pub fn remove_child(&mut self, parent: NodeId, child: NodeId) -> Result<(), DomError>;
    pub fn set_attribute(&mut self, id: NodeId, name: &str, value: &str) -> Result<(), DomError>;
    pub fn remove_attribute(&mut self, id: NodeId, name: &str) -> Result<(), DomError>;
    pub fn set_text(&mut self, id: NodeId, text: &str) -> Result<(), DomError>;
}

// Invalidation (consumed by browser to decide what to recompute)
impl Document {
    pub fn take_mutations(&mut self) -> Vec<Mutation>;
}
pub enum Mutation { ChildrenChanged(NodeId), AttributeChanged(NodeId, String), TextChanged(NodeId) }
```

Names are the proposal; the HTML Team owns the final spelling. The *shape*
(arena, `common::ids::NodeId`, read API separate from mutation API, mutation
log for invalidation) is the contract.

## Rules for consumers

- `css` and `layout` take `&Document` and never hold a `NodeId` across a
  mutation without re-validating it through `node()`.
- `webapis` is the only consumer that calls the mutation API, on behalf of
  scripts. It does so through its own `document` binding, never by exposing
  `Document` to JavaScript directly.
- `devtools` reads only.
- Nobody constructs `Node` values by hand outside `html`; use the mutation
  API so the parent/child invariants hold.

## Selectors

Selector matching is owned by `html` today (`query_selector`) because the
HTML Team built it first. The CSS Team needs the same matcher for the cascade.
Rather than two matchers, the plan is:

1. `css` owns selector **parsing** (`css::Selector`, specificity).
2. `html` exposes the structural predicates a matcher needs (`element`,
   `attribute`, `parent`, `children`, `has_class`).
3. The matcher function (`css::matches(&Document, NodeId, &Selector) -> bool`)
   lives in `css`, and `html::Document::query_selector` becomes a thin call
   into it. This needs `html → css`, which the layering forbids, so instead
   `query_selector` moves to `webapis` (where the DOM API surface for scripts
   lives) or `css` re-exports a `query` helper that `webapis` calls.

This is an open decision between the HTML and CSS teams. Until it is made,
neither team should add a second matcher.

## Current state (2026-10-03)

Read [`../crates/html.md`](../crates/html.md) §Current state for detail. In
summary: the arena exists as `html::HtmlDocument` but uses its own
`html::NodeId(usize)` rather than `common::ids::NodeId`; two older DOM models
(`html::Dom`/`LegacyNode`/`HTMLElement`, and a dead `simple_html_parser.rs`)
sit beside it; there is no mutation API beyond `push_node`/`append_child` and
no mutation log; `QuirksMode` is never computed. Gaps G-01, G-02, G-03.
