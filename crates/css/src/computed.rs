//! Computed declarations for each element after the cascade.

use crate::color::{CssColor, parse_color};
use crate::stylesheet::Declaration;
use html::NodeID;

/// A pseudo-element that generates a box inside its originating element.
///
/// Only `::before` and `::after` are styled. Other pseudo-elements
/// (`::marker`, `::first-line`, `::first-letter`, ...) still never match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PseudoElement {
    /// `::before` (or legacy `:before`): a box before the element's first child.
    Before,
    /// `::after` (or legacy `:after`): a box after the element's last child.
    After,
}

impl PseudoElement {
    /// Parses a pseudo-element name written without colons, ignoring ASCII
    /// case. Returns `None` for names other than `before` and `after`.
    pub fn from_name(name: &str) -> Option<Self> {
        if name.eq_ignore_ascii_case("before") {
            Some(Self::Before)
        } else if name.eq_ignore_ascii_case("after") {
            Some(Self::After)
        } else {
            None
        }
    }
}

/// The result of the cascade: computed declarations for every element, plus
/// for each `::before` / `::after` that at least one rule targets.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ComputedStyles {
    /// Computed declarations per element, in document order.
    pub styles: Vec<(NodeID, Vec<Declaration>)>,
    /// Computed declarations per pseudo-element, sorted by node then
    /// pseudo-element. Read through [`ComputedStyles::pseudo_declarations_for`].
    pseudo: Vec<((NodeID, PseudoElement), Vec<Declaration>)>,
}

impl ComputedStyles {
    /// Returns the computed declarations for an element, or `None` for nodes
    /// that are not elements.
    ///
    /// Entries are stored in document order, so lookup is a binary search.
    pub fn declarations_for(&self, node: NodeID) -> Option<&[Declaration]> {
        computed_style(&self.styles, node).map(Vec::as_slice)
    }

    /// Returns the computed declarations of `node`'s `::before` or `::after`.
    ///
    /// Returns `None` when no rule targets that pseudo-element. A returned
    /// style inherits from the element, as the cascade's inheritance rules
    /// describe. Whether a box is generated is up to the caller: per CSS
    /// Generated Content, a `content` of `none` or `normal` (or no `content`
    /// declaration) generates no box.
    pub fn pseudo_declarations_for(
        &self,
        node: NodeID,
        pseudo: PseudoElement,
    ) -> Option<&[Declaration]> {
        self.pseudo
            .binary_search_by_key(&(node.get(), pseudo), |((id, which), _)| (id.get(), *which))
            .ok()
            .map(|position| self.pseudo[position].1.as_slice())
    }

    /// Builds styles from element and pseudo-element entries. Both must be
    /// sorted by node, then pseudo-element.
    pub(crate) fn new(
        styles: Vec<(NodeID, Vec<Declaration>)>,
        pseudo: Vec<((NodeID, PseudoElement), Vec<Declaration>)>,
    ) -> Self {
        Self { styles, pseudo }
    }

    /// Returns the computed value text of one property for an element.
    pub fn value_for(&self, node: NodeID, property: &str) -> Option<&str> {
        self.declarations_for(node)?
            .iter()
            .find(|declaration| declaration.property == property)
            .map(|declaration| declaration.value.as_str())
    }

    /// Returns a parsed color property for an HTML node after the cascade.
    ///
    /// `property` is typically `color` or `background-color`. Unsupported
    /// color syntax and absent declarations return `None`.
    pub fn color_for(&self, node: NodeID, property: &str) -> Option<CssColor> {
        self.value_for(node, property).and_then(parse_color)
    }
}

pub(crate) fn computed_style(
    styles: &[(NodeID, Vec<Declaration>)],
    parent: NodeID,
) -> Option<&Vec<Declaration>> {
    styles
        .binary_search_by_key(&parent.get(), |(id, _)| id.get())
        .ok()
        .map(|position| &styles[position].1)
}
