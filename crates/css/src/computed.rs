//! Computed declarations for each element after the cascade.

use crate::color::{CssColor, parse_color};
use crate::stylesheet::Declaration;
use html::NodeID;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ComputedStyles {
    pub styles: Vec<(NodeID, Vec<Declaration>)>,
}

impl ComputedStyles {
    /// Returns the computed declarations for an element, or `None` for nodes
    /// that are not elements.
    ///
    /// Entries are stored in document order, so lookup is a binary search.
    pub fn declarations_for(&self, node: NodeID) -> Option<&[Declaration]> {
        computed_style(&self.styles, node).map(Vec::as_slice)
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
