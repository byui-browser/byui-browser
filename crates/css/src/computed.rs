//! Computed declarations for each element after the cascade.

use crate::color::{CssColor, parse_color};
use crate::stylesheet::Declaration;
use html::NodeID;

/// The computed declarations of every element in a document.
///
/// Produced by [`crate::style_document`] and [`crate::style_document_at`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ComputedStyles {
    /// Each element's computed declarations, in document order. Values are
    /// text; `font-size` is always an absolute length in CSS pixels.
    pub styles: Vec<(NodeID, Vec<Declaration>)>,
    /// How many CSS animations are delayed or still playing at the styled
    /// moment, so styling a later moment can give different results. Zero
    /// means the styles are stable until the document or stylesheets change;
    /// a renderer can stop requesting animation frames.
    pub running_animations: usize,
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
