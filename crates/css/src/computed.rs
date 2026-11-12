//! Computed declarations for each element after the cascade.

use crate::color::{CssColor, parse_color};
use crate::stylesheet::Declaration;
use html::NodeID;

/// A tree-abiding pseudo-element that generates content next to an element's
/// children (CSS Pseudo-Elements 4 §4.1).
///
/// Selectors may spell these `::before`/`::after` or, with the CSS 2 legacy
/// syntax, `:before`/`:after`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PseudoElement {
    /// `::before`: generated content placed before the element's children.
    Before,
    /// `::after`: generated content placed after the element's children.
    After,
}

impl PseudoElement {
    /// Both pseudo-elements, in tree order.
    pub const ALL: [Self; 2] = [Self::Before, Self::After];

    /// The pseudo-element a selector name refers to, without colons and
    /// compared ASCII case-insensitively, or `None` for other names.
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

/// The computed declarations of every element in a document.
///
/// Produced by [`crate::style_document`] and [`crate::style_document_at`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ComputedStyles {
    /// Each element's computed declarations, in document order. Values are
    /// text; `font-size` is always an absolute length in CSS pixels.
    pub styles: Vec<(NodeID, Vec<Declaration>)>,
    /// The computed declarations of every generated `::before` and `::after`
    /// pseudo-element, ordered by originating element and then
    /// [`PseudoElement`] (`Before` first). An entry exists only when the
    /// pseudo-element generates a box: its computed `content` is neither
    /// `none` nor `normal`. Look entries up with
    /// [`ComputedStyles::pseudo_declarations_for`].
    pub pseudo_elements: Vec<(NodeID, PseudoElement, Vec<Declaration>)>,
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

    /// Returns the computed declarations of `node`'s `pseudo` pseudo-element,
    /// or `None` when the element does not generate one.
    ///
    /// The declarations include a `content` declaration holding the specified
    /// value; turn it into text with [`crate::generated_content_text`].
    /// Inherited properties come from the originating element.
    pub fn pseudo_declarations_for(
        &self,
        node: NodeID,
        pseudo: PseudoElement,
    ) -> Option<&[Declaration]> {
        self.pseudo_elements
            .binary_search_by_key(&(node.get(), pseudo), |(id, pseudo, _)| (id.get(), *pseudo))
            .ok()
            .map(|position| self.pseudo_elements[position].2.as_slice())
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
