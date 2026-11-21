//! Element-scoped tree queries and mutations backing the `Element` bindings.
//!
//! These [`Document`] methods implement the tree algorithms behind
//! `Element.querySelectorAll()`, `removeChild()`, `replaceChild()`,
//! `replaceChildren()`, `before()`, `outerHTML`, and the
//! `clientWidth`/`clientHeight` getters. They operate on the document's HTML
//! arena by [`NodeID`], so nodes keep their stable IDs when they move.

use html::{HTMLDocument, NodeID, NodeKind};

use crate::document::{Document, collect_elements};
use crate::dom_exception::DomException;
use crate::selector::{SelectorError, SimpleSelector, parse_selector_list};

/// One argument to a DOM method that accepts `(Node or DOMString)...`, such as
/// `Element.before()` and `Element.replaceChildren()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeInput {
    /// An existing node in the document's arena. Inserting it first removes
    /// it from its current parent, as the DOM does when a node moves.
    Node(NodeID),
    /// A string, which the DOM converts into a new `Text` node holding this
    /// data before inserting it.
    Text(String),
}

/// The size of an element's client area (its padding box, excluding
/// scrollbars), in CSS pixels, as reported by `Element.clientWidth` and
/// `Element.clientHeight`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ClientSize {
    /// Client width in CSS pixels, rounded to an integer as the CSSOM View
    /// getters require.
    pub width: f64,
    /// Client height in CSS pixels, rounded to an integer as the CSSOM View
    /// getters require.
    pub height: f64,
}

impl Document {
    /// Implements `Element.querySelectorAll(selectors)` for the element
    /// `root`.
    ///
    /// Returns a static snapshot of the matching descendants of `root` in tree
    /// order; `root` itself is never a candidate. Supported selector syntax is
    /// the same as [`Document::query_selector_all`]: type selectors, `*`, IDs,
    /// classes, compound combinations of those, and comma-separated lists.
    /// Selectors are matched against each element alone, so (unlike the DOM)
    /// no combinator can reach outside `root` because combinators are not
    /// supported. An invalid or unsupported selector returns a
    /// [`SelectorError`], which bindings surface as a `SyntaxError`.
    pub fn query_selector_all_within(
        &self,
        root: NodeID,
        selector: &str,
    ) -> Result<Vec<NodeID>, SelectorError> {
        log::debug!("Element.querySelectorAll({selector:?})");
        let selectors = parse_selector_list(selector)?;
        let mut matches = Vec::new();
        let Some(document) = &self.source_document else {
            return Ok(matches);
        };

        fn collect(
            document: &HTMLDocument,
            parent: NodeID,
            selectors: &[SimpleSelector],
            matches: &mut Vec<NodeID>,
        ) {
            let Some(parent_node) = document.get_node(parent) else {
                return;
            };
            for child in parent_node.get_children() {
                let Some(node) = document.get_node(*child) else {
                    continue;
                };
                if let NodeKind::Element(element) = node.get_kind() {
                    if selectors.iter().any(|selector| selector.matches(element)) {
                        matches.push(*child);
                    }
                    collect(document, *child, selectors, matches);
                }
            }
        }

        collect(document, root, &selectors, &mut matches);
        Ok(matches)
    }

    /// Implements `Node.removeChild(child)`.
    ///
    /// Removes `child` from `parent` and returns it. The removed subtree stays
    /// in the arena with its node IDs, detached from the document. Returns
    /// [`DomException::NotFoundError`] when `child` is not a child of
    /// `parent`.
    pub fn remove_child(&mut self, parent: NodeID, child: NodeID) -> Result<NodeID, DomException> {
        let source = self
            .source_document
            .as_mut()
            .ok_or(DomException::NotFoundError)?;
        if source.get_node(child).and_then(|node| node.get_parent()) != Some(parent) {
            log::debug!("removeChild failed: node is not a child of this parent");
            return Err(DomException::NotFoundError);
        }
        source.remove(child);
        self.elements = collect_elements(source);
        Ok(child)
    }

    /// Implements `Node.replaceChild(node, child)`.
    ///
    /// Puts `node` where `child` is in `parent`'s children and returns
    /// `child`, which is left detached. If `node` already has a parent it is
    /// moved. Replacing a child with itself leaves the tree unchanged.
    ///
    /// Returns [`DomException::NotFoundError`] when `child` is not a child of
    /// `parent` or a node ID is unknown, and
    /// [`DomException::HierarchyRequestError`] when `node` is an inclusive
    /// ancestor of `parent` or the result would break the document's child
    /// rules (for example a second document element, or text at the document
    /// level).
    pub fn replace_child(
        &mut self,
        parent: NodeID,
        node: NodeID,
        child: NodeID,
    ) -> Result<NodeID, DomException> {
        self.ensure_pre_insert_validity(parent, &[node], false, &[child])?;
        let source = self.source_document.as_mut().unwrap();
        if source.get_node(child).and_then(|data| data.get_parent()) != Some(parent) {
            log::debug!("replaceChild failed: child is not a child of this parent");
            return Err(DomException::NotFoundError);
        }
        if node == child {
            return Ok(child);
        }
        source.remove(node);
        let mut children = source.get_node(parent).unwrap().get_children().clone();
        let Some(position) = children.iter().position(|sibling| *sibling == child) else {
            return Err(DomException::NotFoundError);
        };
        children[position] = node;
        source.get_mut_node(parent).unwrap().set_children(children);
        source.get_mut_node(node).unwrap().set_parent(Some(parent));
        source.get_mut_node(child).unwrap().set_parent(None);
        self.elements = collect_elements(source);
        Ok(child)
    }

    /// Implements `ParentNode.replaceChildren(...nodes)`.
    ///
    /// Removes every child of `parent` and then appends `nodes` in order.
    /// [`NodeInput::Text`] values become new text nodes. A node listed more
    /// than once ends up at its last position, as when the DOM gathers the
    /// arguments into a `DocumentFragment`. On error the tree is unchanged;
    /// errors are the same as for [`Document::replace_child`].
    pub fn replace_children(
        &mut self,
        parent: NodeID,
        nodes: &[NodeInput],
    ) -> Result<(), DomException> {
        let (ids, has_text) = split_inputs(nodes);
        let current_children = self
            .source_document
            .as_ref()
            .and_then(|source| source.get_node(parent))
            .map(|node| node.get_children().clone())
            .unwrap_or_default();
        self.ensure_pre_insert_validity(parent, &ids, has_text, &current_children)?;
        let new_children = self.materialize(nodes);
        let source = self.source_document.as_mut().unwrap();
        for old_child in current_children {
            source.get_mut_node(old_child).unwrap().set_parent(None);
        }
        source
            .get_mut_node(parent)
            .unwrap()
            .set_children(Vec::new());
        for node in &new_children {
            source.remove(*node);
            source.get_mut_node(*node).unwrap().set_parent(Some(parent));
        }
        source
            .get_mut_node(parent)
            .unwrap()
            .set_children(new_children);
        self.elements = collect_elements(source);
        Ok(())
    }

    /// Implements `ChildNode.before(...nodes)` for `node`.
    ///
    /// Inserts `nodes` into `node`'s parent immediately before `node`, in
    /// argument order. [`NodeInput::Text`] values become new text nodes. When
    /// `node` has no parent this is a no-op, as in the DOM. Nodes passed that
    /// are already earlier siblings of `node` are moved rather than used as
    /// the insertion point, following the DOM's "viable previous sibling"
    /// rule. Errors are the same as for [`Document::replace_child`].
    pub fn before(&mut self, node: NodeID, nodes: &[NodeInput]) -> Result<(), DomException> {
        let Some(source) = &self.source_document else {
            return Ok(());
        };
        let Some(parent) = source.get_node(node).and_then(|data| data.get_parent()) else {
            return Ok(());
        };
        let (ids, has_text) = split_inputs(nodes);
        let siblings = source.get_node(parent).unwrap().get_children().clone();
        let position = siblings
            .iter()
            .position(|sibling| *sibling == node)
            .ok_or(DomException::NotFoundError)?;
        let viable_previous = siblings[..position]
            .iter()
            .rev()
            .find(|sibling| !ids.contains(sibling))
            .copied();
        self.ensure_pre_insert_validity(parent, &ids, has_text, &[])?;
        let inserted = self.materialize(nodes);
        let source = self.source_document.as_mut().unwrap();
        for id in &inserted {
            source.remove(*id);
        }
        let mut children = source.get_node(parent).unwrap().get_children().clone();
        let index = viable_previous
            .and_then(|previous| children.iter().position(|sibling| *sibling == previous))
            .map_or(0, |index| index + 1);
        for (offset, id) in inserted.iter().enumerate() {
            children.insert(index + offset, *id);
            source.get_mut_node(*id).unwrap().set_parent(Some(parent));
        }
        source.get_mut_node(parent).unwrap().set_children(children);
        self.elements = collect_elements(source);
        Ok(())
    }

    /// Implements `ChildNode.after(...nodes)` for `node`.
    ///
    /// Inserts nodes immediately after `node` in argument order. Detached
    /// nodes are a no-op; existing nodes are moved according to the DOM's
    /// viable-next-sibling rule.
    pub fn after(&mut self, node: NodeID, nodes: &[NodeInput]) -> Result<(), DomException> {
        let Some(source) = &self.source_document else {
            return Ok(());
        };
        let Some(parent) = source.get_node(node).and_then(|data| data.get_parent()) else {
            return Ok(());
        };
        let (ids, has_text) = split_inputs(nodes);
        let siblings = source.get_node(parent).unwrap().get_children().clone();
        let position = siblings
            .iter()
            .position(|sibling| *sibling == node)
            .ok_or(DomException::NotFoundError)?;
        let viable_next = siblings[position + 1..]
            .iter()
            .find(|sibling| !ids.contains(sibling))
            .copied();
        self.ensure_pre_insert_validity(parent, &ids, has_text, &[])?;
        let inserted = self.materialize(nodes);
        let source = self.source_document.as_mut().unwrap();
        for id in &inserted {
            source.remove(*id);
        }
        let mut children = source.get_node(parent).unwrap().get_children().clone();
        let index = viable_next
            .and_then(|next| children.iter().position(|sibling| *sibling == next))
            .unwrap_or(children.len());
        for (offset, id) in inserted.iter().enumerate() {
            children.insert(index + offset, *id);
            source.get_mut_node(*id).unwrap().set_parent(Some(parent));
        }
        source.get_mut_node(parent).unwrap().set_children(children);
        self.elements = collect_elements(source);
        Ok(())
    }

    /// Implements `ParentNode.append(...nodes)` for `parent`.
    ///
    /// Appends `nodes` after `parent`'s existing children, in argument order.
    /// [`NodeInput::Text`] values become new text nodes. A node that already
    /// has a parent is moved. A node listed more than once ends up at its last
    /// position, as when the DOM gathers the arguments into a
    /// `DocumentFragment`. An empty list changes nothing. On error the tree
    /// is unchanged; errors are the same as for [`Document::replace_child`].
    pub fn append(&mut self, parent: NodeID, nodes: &[NodeInput]) -> Result<(), DomException> {
        if nodes.is_empty() {
            return Ok(());
        }
        let (ids, has_text) = split_inputs(nodes);
        self.ensure_pre_insert_validity(parent, &ids, has_text, &[])?;
        let inserted = self.materialize(nodes);
        let source = self.source_document.as_mut().unwrap();
        for id in &inserted {
            source.remove(*id);
        }
        let mut children = source.get_node(parent).unwrap().get_children().clone();
        for id in &inserted {
            children.push(*id);
            source.get_mut_node(*id).unwrap().set_parent(Some(parent));
        }
        source.get_mut_node(parent).unwrap().set_children(children);
        self.elements = collect_elements(source);
        log::debug!("Element.append inserted {} nodes", inserted.len());
        Ok(())
    }

    /// Implements the `Element.outerHTML` getter.
    ///
    /// Serializes `node` and its descendants with the HTML fragment
    /// serialization rules of [`HTMLDocument::outer_html`], including its
    /// limitations. Returns `None` for an unknown node ID.
    pub fn outer_html(&self, node: NodeID) -> Option<String> {
        self.source_document.as_ref()?.outer_html(node)
    }

    /// Implements the `Element.clientWidth` and `Element.clientHeight`
    /// getters for `node`.
    ///
    /// Limitation: script bindings have no access to layout results, so the
    /// size is derived from the cascaded `width`/`height` when they are
    /// absolute `px` lengths: the padding box is the content size plus
    /// `padding-*`, or, with `box-sizing: border-box`, the declared size minus
    /// the `border-*-width` longhands. Auto, percentage, and other unit sizes
    /// report `0`. As the CSSOM View spec requires, detached elements and
    /// elements with `display: none` or `display: inline` also report `0`.
    /// An element with no cascaded `display` value is treated as a block box.
    pub fn client_size(&self, node: NodeID) -> ClientSize {
        if !self.is_connected(node) {
            return ClientSize::default();
        }
        let styles = self.computed_styles();
        let value = |property: &str| styles.value_for(node, property);
        if matches!(value("display"), Some("none" | "inline")) {
            return ClientSize::default();
        }
        let length = |property: &str| value(property).and_then(parse_px).unwrap_or(0.0);
        let border_box = value("box-sizing") == Some("border-box");
        let axis = |size: &str, start: &str, end: &str| {
            let Some(size) = value(size).and_then(parse_px) else {
                return 0.0;
            };
            let client = if border_box {
                size - length(&format!("border-{start}-width"))
                    - length(&format!("border-{end}-width"))
            } else {
                size + length(&format!("padding-{start}")) + length(&format!("padding-{end}"))
            };
            client.max(0.0).round()
        };
        ClientSize {
            width: axis("width", "left", "right"),
            height: axis("height", "top", "bottom"),
        }
    }

    /// Checks the DOM's pre-insertion validity for inserting `nodes` (plus
    /// new text nodes when `has_text`) into `parent`, ignoring `replaced`
    /// children that the operation removes.
    fn ensure_pre_insert_validity(
        &self,
        parent: NodeID,
        nodes: &[NodeID],
        has_text: bool,
        replaced: &[NodeID],
    ) -> Result<(), DomException> {
        let source = self
            .source_document
            .as_ref()
            .ok_or(DomException::NotFoundError)?;
        let parent_node = source.get_node(parent).ok_or(DomException::NotFoundError)?;
        let parent_is_document = match parent_node.get_kind() {
            NodeKind::Document => true,
            NodeKind::Element(_) => false,
            _ => return Err(DomException::HierarchyRequestError),
        };
        for node in nodes {
            let data = source.get_node(*node).ok_or(DomException::NotFoundError)?;
            let mut ancestor = Some(parent);
            while let Some(current) = ancestor {
                if current == *node {
                    log::debug!("insertion failed: node is an inclusive ancestor of the parent");
                    return Err(DomException::HierarchyRequestError);
                }
                ancestor = source.get_node(current).and_then(|data| data.get_parent());
            }
            match data.get_kind() {
                NodeKind::Element(_) | NodeKind::Comment(_) => {}
                NodeKind::Text(_) if !parent_is_document => {}
                NodeKind::Doctype(_) if parent_is_document => {}
                _ => return Err(DomException::HierarchyRequestError),
            }
        }
        if parent_is_document {
            if has_text {
                return Err(DomException::HierarchyRequestError);
            }
            let remaining_elements = parent_node
                .get_children()
                .iter()
                .filter(|child| !replaced.contains(child) && !nodes.contains(child))
                .filter(|child| {
                    matches!(
                        source.get_node(**child).map(|data| data.get_kind()),
                        Some(NodeKind::Element(_))
                    )
                })
                .count();
            let distinct_elements = {
                let mut seen = Vec::new();
                for node in nodes {
                    if !seen.contains(node)
                        && matches!(
                            source.get_node(*node).map(|data| data.get_kind()),
                            Some(NodeKind::Element(_))
                        )
                    {
                        seen.push(*node);
                    }
                }
                seen.len()
            };
            if remaining_elements + distinct_elements > 1 {
                log::debug!("insertion failed: document would have two element children");
                return Err(DomException::HierarchyRequestError);
            }
        }
        Ok(())
    }

    /// Creates text nodes for string inputs and returns the node list to
    /// insert, keeping only the last occurrence of a repeated node.
    fn materialize(&mut self, nodes: &[NodeInput]) -> Vec<NodeID> {
        let source = self.source_document.get_or_insert_with(HTMLDocument::new);
        let ids: Vec<NodeID> = nodes
            .iter()
            .map(|input| match input {
                NodeInput::Node(id) => *id,
                NodeInput::Text(text) => source.push_node(NodeKind::Text(text.clone())),
            })
            .collect();
        ids.iter()
            .enumerate()
            .filter(|(index, id)| !ids[index + 1..].contains(id))
            .map(|(_, id)| *id)
            .collect()
    }
}

fn split_inputs(nodes: &[NodeInput]) -> (Vec<NodeID>, bool) {
    let mut ids = Vec::new();
    let mut has_text = false;
    for input in nodes {
        match input {
            NodeInput::Node(id) => ids.push(*id),
            NodeInput::Text(_) => has_text = true,
        }
    }
    (ids, has_text)
}

/// Parses an absolute pixel length such as `12px`, `12.5px`, or `0`.
fn parse_px(value: &str) -> Option<f64> {
    let value = value.trim();
    if value == "0" {
        return Some(0.0);
    }
    value.strip_suffix("px")?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use html::parse_raw_html;

    use super::*;

    fn document(markup: &str) -> Document {
        Document::from_html_document(&parse_raw_html(markup.to_owned()))
    }

    fn id(document: &Document, id: &str) -> NodeID {
        document.get_element_by_id(id).expect(id).node
    }

    fn child_ids(document: &Document, parent: NodeID) -> Vec<String> {
        let source = document.html_document().unwrap();
        source
            .get_node(parent)
            .unwrap()
            .get_children()
            .iter()
            .map(|child| match source.get_node(*child).unwrap().get_kind() {
                NodeKind::Element(element) => element
                    .get_attributes()
                    .iter()
                    .find(|attribute| attribute.get_name() == "id")
                    .map_or_else(String::new, |attribute| attribute.get_value().clone()),
                NodeKind::Text(text) => format!("#{text}"),
                _ => "?".to_owned(),
            })
            .collect()
    }

    #[test]
    fn query_selector_all_within_excludes_root_and_outside_elements() {
        let document = document(
            "<div id='root' class='x'><p class='x' id='a'><b class='x' id='b'></b></p></div>\
             <p class='x' id='outside'></p>",
        );
        let root = id(&document, "root");
        let matches = document.query_selector_all_within(root, ".x").unwrap();
        assert_eq!(matches, [id(&document, "a"), id(&document, "b")]);
        assert!(document.query_selector_all_within(root, "p >").is_err());
    }

    #[test]
    fn remove_child_requires_a_direct_child() {
        let mut document = document("<div id='p'><span id='c'><i id='g'></i></span></div>");
        let (parent, child, grandchild) =
            (id(&document, "p"), id(&document, "c"), id(&document, "g"));
        assert_eq!(
            document.remove_child(parent, grandchild),
            Err(DomException::NotFoundError)
        );
        assert_eq!(document.remove_child(parent, child), Ok(child));
        assert!(child_ids(&document, parent).is_empty());
        assert!(!document.is_connected(child));
    }

    #[test]
    fn replace_child_swaps_in_place_and_rejects_ancestors() {
        let mut document =
            document("<div id='p'><a id='one'></a><a id='two'></a></div><b id='new'></b>");
        let parent = id(&document, "p");
        let one = id(&document, "one");
        let new = id(&document, "new");
        assert_eq!(document.replace_child(parent, new, one), Ok(one));
        assert_eq!(child_ids(&document, parent), ["new", "two"]);
        assert!(!document.is_connected(one));
        assert_eq!(
            document.replace_child(new, parent, new),
            Err(DomException::HierarchyRequestError)
        );
        assert_eq!(
            document.replace_child(parent, one, one),
            Err(DomException::NotFoundError)
        );
    }

    #[test]
    fn replace_children_replaces_with_nodes_and_text() {
        let mut document = document("<div id='p'><a id='one'></a><a id='two'></a></div>");
        let parent = id(&document, "p");
        let two = id(&document, "two");
        document
            .replace_children(
                parent,
                &[
                    NodeInput::Text("hi".into()),
                    NodeInput::Node(two),
                    NodeInput::Text("bye".into()),
                ],
            )
            .unwrap();
        assert_eq!(child_ids(&document, parent), ["#hi", "two", "#bye"]);
        assert!(document.get_element_by_id("one").is_none());
        document.replace_children(parent, &[]).unwrap();
        assert!(child_ids(&document, parent).is_empty());
    }

    #[test]
    fn before_inserts_ahead_of_the_node_using_the_viable_previous_sibling() {
        let mut document =
            document("<div id='p'><a id='one'></a><a id='two'></a><a id='three'></a></div>");
        let parent = id(&document, "p");
        let one = id(&document, "one");
        let two = id(&document, "two");
        let three = id(&document, "three");
        document
            .before(three, &[NodeInput::Node(two), NodeInput::Text("t".into())])
            .unwrap();
        assert_eq!(child_ids(&document, parent), ["one", "two", "#t", "three"]);
        document.before(two, &[NodeInput::Node(three)]).unwrap();
        assert_eq!(child_ids(&document, parent), ["one", "three", "two", "#t"]);
        document.before(one, &[NodeInput::Node(one)]).unwrap();
        assert_eq!(child_ids(&document, parent), ["one", "three", "two", "#t"]);

        let mut detached = Document::new();
        let lonely = detached.create_element("p").unwrap().node;
        assert_eq!(
            detached.before(lonely, &[NodeInput::Text("x".into())]),
            Ok(())
        );
    }

    #[test]
    fn client_size_uses_px_sizes_and_padding_and_is_zero_otherwise() {
        let document = document(
            "<div id='box' style='width: 100px; height: 40px; padding: 5px 10px'></div>\
             <div id='bb' style='box-sizing: border-box; width: 100px; height: 50px; \
             border-left-width: 2px; border-right-width: 3px'></div>\
             <div id='auto'></div><span id='inline' style='display: inline; width: 10px'></span>\
             <div id='hidden' style='display: none; width: 10px'></div>",
        );
        assert_eq!(
            document.client_size(id(&document, "box")),
            ClientSize {
                width: 120.0,
                height: 50.0
            }
        );
        assert_eq!(
            document.client_size(id(&document, "bb")),
            ClientSize {
                width: 95.0,
                height: 50.0
            }
        );
        for zero in ["auto", "inline", "hidden"] {
            assert_eq!(
                document.client_size(id(&document, zero)),
                ClientSize::default(),
                "{zero}"
            );
        }
    }

    #[test]
    fn outer_html_serializes_the_element_and_its_subtree() {
        let document = document("<ul id='list'><li class='a'>x &amp; y</li></ul>");
        assert_eq!(
            document.outer_html(id(&document, "list")).as_deref(),
            Some("<ul id=\"list\"><li class=\"a\">x &amp; y</li></ul>")
        );
    }

    #[test]
    fn append_adds_nodes_and_text_and_moves_existing_children() {
        let mut document = document("<div id='p'><a id='one'></a></div><b id='two'></b>");
        let parent = id(&document, "p");
        let one = id(&document, "one");
        let two = id(&document, "two");
        document
            .append(
                parent,
                &[NodeInput::Text("hi".into()), NodeInput::Node(two)],
            )
            .unwrap();
        assert_eq!(child_ids(&document, parent), ["one", "#hi", "two"]);
        document.append(parent, &[NodeInput::Node(one)]).unwrap();
        assert_eq!(child_ids(&document, parent), ["#hi", "two", "one"]);
        document.append(parent, &[]).unwrap();
        assert_eq!(child_ids(&document, parent), ["#hi", "two", "one"]);
        assert_eq!(
            document.append(two, &[NodeInput::Node(parent)]),
            Err(DomException::HierarchyRequestError)
        );
    }
}
