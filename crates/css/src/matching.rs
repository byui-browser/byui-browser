//! Matching parsed selectors against elements of the HTML DOM.
//!
//! [`ElementTree`] precomputes element-only parent and sibling links in one
//! pass over the arena, so combinators and structural pseudo-classes do not
//! rescan child lists. Matching runs right to left with backtracking, the
//! usual browser strategy (Selectors 4 §17).
//!
//! The engine has no interaction or history state, so dynamic pseudo-classes
//! (`:hover`, `:focus`, `:visited`, ...) never match, and pseudo-elements
//! (`::before`) never match the element itself; the cascade removes a
//! trailing `::before` or `::after` before matching and styles that
//! pseudo-element instead. Unknown pseudo-classes never match. Namespace prefixes other than `*` and the default match nothing,
//! because `@namespace` is not supported.

use crate::selector::{
    AttributeOperator, AttributeSelector, Combinator, ComplexSelector, CompoundSelector,
    PseudoSelector, SelectorList, SimpleSelector, TypeSelector,
};
use crate::specificity::is_legacy_pseudo_element;
use html::{ElementData, HTMLDocument, Namespace, NodeKind};
use std::borrow::Cow;

/// Pseudo-classes for state the engine does not track. They never match, so
/// rules that require them can be skipped when the rule index is built.
const NEVER_MATCHING_PSEUDO_CLASSES: &[&str] = &[
    "active",
    "autofill",
    "-webkit-autofill",
    "blank",
    "buffering",
    "current",
    "default",
    "focus",
    "focus-visible",
    "focus-within",
    "fullscreen",
    "future",
    "host",
    "host-context",
    "hover",
    "in-range",
    "indeterminate",
    "invalid",
    "local-link",
    "modal",
    "muted",
    "out-of-range",
    "past",
    "paused",
    "picture-in-picture",
    "playing",
    "popover-open",
    "seeking",
    "stalled",
    "target",
    "target-within",
    "user-invalid",
    "user-valid",
    "valid",
    "visited",
    "volume-locked",
];

/// HTML attributes whose values selectors compare ASCII case-insensitively
/// (HTML Standard §4.16.2, "Case-sensitivity of selectors"). Kept sorted for
/// binary search.
const CASE_INSENSITIVE_HTML_ATTRIBUTES: &[&str] = &[
    "accept",
    "accept-charset",
    "align",
    "alink",
    "axis",
    "bgcolor",
    "charset",
    "checked",
    "clear",
    "codetype",
    "color",
    "compact",
    "declare",
    "defer",
    "dir",
    "direction",
    "disabled",
    "enctype",
    "face",
    "frame",
    "hreflang",
    "http-equiv",
    "lang",
    "language",
    "link",
    "media",
    "method",
    "multiple",
    "nohref",
    "noresize",
    "noshade",
    "nowrap",
    "readonly",
    "rel",
    "rev",
    "rules",
    "scope",
    "scrolling",
    "selected",
    "shape",
    "target",
    "text",
    "type",
    "valign",
    "valuetype",
    "vlink",
];

const FORM_CONTROLS: &[&str] = &[
    "button", "fieldset", "input", "optgroup", "option", "select", "textarea",
];

fn lowercase(name: &str) -> Cow<'_, str> {
    if name.bytes().any(|byte| byte.is_ascii_uppercase()) {
        Cow::Owned(name.to_ascii_lowercase())
    } else {
        Cow::Borrowed(name)
    }
}

/// Whether `compound` can never match any element in this engine, either
/// because it targets a pseudo-element or needs untracked dynamic state.
pub(crate) fn compound_never_matches(compound: &CompoundSelector) -> bool {
    compound.simples.iter().any(|simple| match simple {
        SimpleSelector::Pseudo(pseudo) => {
            let name = lowercase(&pseudo.name);
            pseudo.element
                || is_legacy_pseudo_element(&name)
                || NEVER_MATCHING_PSEUDO_CLASSES.contains(&name.as_ref())
        }
        _ => false,
    })
}

/// Returns the value of attribute `name` on `element`.
pub(crate) fn attribute<'e>(element: &'e ElementData, name: &str) -> Option<&'e str> {
    element
        .get_attributes()
        .iter()
        .find(|attribute| attribute.get_name() == name)
        .map(|attribute| attribute.get_value().as_str())
}

/// Element-only tree links for one document, indexed by node arena index.
pub(crate) struct ElementTree<'d> {
    dom: &'d HTMLDocument,
    /// Parent element; `None` for the root element and non-elements.
    parent: Vec<Option<usize>>,
    /// Previous sibling that is an element.
    previous: Vec<Option<usize>>,
    /// Next sibling that is an element.
    next: Vec<Option<usize>>,
    /// Whether the node's parent is the document node.
    is_root: Vec<bool>,
}

impl<'d> ElementTree<'d> {
    /// Builds the element links for `dom` in one pass over its nodes.
    pub(crate) fn new(dom: &'d HTMLDocument) -> Self {
        let count = dom.nodes.len();
        let mut tree = Self {
            dom,
            parent: vec![None; count],
            previous: vec![None; count],
            next: vec![None; count],
            is_root: vec![false; count],
        };
        for (index, node) in dom.nodes.iter().enumerate() {
            let parent_is_element = matches!(node.get_kind(), NodeKind::Element(_));
            let parent_is_document = matches!(node.get_kind(), NodeKind::Document);
            let mut previous: Option<usize> = None;
            for child in node.get_children() {
                let child = child.get();
                if child >= count || tree.element(child).is_none() {
                    continue;
                }
                if parent_is_element {
                    tree.parent[child] = Some(index);
                }
                tree.is_root[child] = parent_is_document;
                tree.previous[child] = previous;
                if let Some(previous) = previous {
                    tree.next[previous] = Some(child);
                }
                previous = Some(child);
            }
        }
        tree
    }

    /// The element data of node `index`, or `None` for non-elements.
    pub(crate) fn element(&self, index: usize) -> Option<&'d ElementData> {
        match self.dom.nodes.get(index)?.get_kind() {
            NodeKind::Element(element) => Some(element),
            _ => None,
        }
    }

    fn children(&self, index: usize) -> impl Iterator<Item = usize> + '_ {
        self.dom.nodes[index]
            .get_children()
            .iter()
            .map(|child| child.get())
            .filter(|child| self.element(*child).is_some())
    }

    fn ancestors(&self, index: usize) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(self.parent[index], |current| self.parent[*current])
    }

    fn previous_siblings(&self, index: usize) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(self.previous[index], |current| self.previous[*current])
    }

    fn next_siblings(&self, index: usize) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(self.next[index], |current| self.next[*current])
    }

    /// Calls `visit` on each element descendant of `index` until it returns
    /// `true`, and reports whether it did.
    fn any_descendant(&self, index: usize, visit: &mut impl FnMut(usize) -> bool) -> bool {
        let mut stack: Vec<usize> = self.children(index).collect();
        stack.reverse();
        while let Some(current) = stack.pop() {
            if visit(current) {
                return true;
            }
            let start = stack.len();
            stack.extend(self.children(current));
            stack[start..].reverse();
        }
        false
    }

    /// Whether `selector` matches element `index`.
    pub(crate) fn matches(&self, selector: &ComplexSelector, index: usize) -> bool {
        let Some(last) = selector.compounds.len().checked_sub(1) else {
            return false;
        };
        self.match_from(selector, last, index, None)
    }

    fn matches_any(&self, list: &SelectorList, index: usize) -> bool {
        list.selectors
            .iter()
            .any(|selector| self.matches(selector, index))
    }

    /// Matches `selector.compounds[..=position]` with the compound at
    /// `position` on element `index`. For relative selectors (`:has()`),
    /// `anchor` is the element the leftmost compound must relate to.
    fn match_from(
        &self,
        selector: &ComplexSelector,
        position: usize,
        index: usize,
        anchor: Option<usize>,
    ) -> bool {
        if !self.matches_compound(&selector.compounds[position], index) {
            return false;
        }
        if position == 0 {
            return match anchor {
                None => true,
                Some(anchor) => self.relates(
                    selector
                        .leading_combinator
                        .unwrap_or(Combinator::Descendant),
                    anchor,
                    index,
                ),
            };
        }
        let next = position - 1;
        match selector.combinators[next] {
            Combinator::Child => self.parent[index]
                .is_some_and(|parent| self.match_from(selector, next, parent, anchor)),
            Combinator::Descendant => self
                .ancestors(index)
                .any(|ancestor| self.match_from(selector, next, ancestor, anchor)),
            Combinator::NextSibling => self.previous[index]
                .is_some_and(|sibling| self.match_from(selector, next, sibling, anchor)),
            Combinator::SubsequentSibling => self
                .previous_siblings(index)
                .any(|sibling| self.match_from(selector, next, sibling, anchor)),
            // Table column membership is not modeled.
            Combinator::Column => false,
        }
    }

    /// Whether `subject` is related to `anchor` by `combinator`.
    fn relates(&self, combinator: Combinator, anchor: usize, subject: usize) -> bool {
        match combinator {
            Combinator::Descendant => self.ancestors(subject).any(|ancestor| ancestor == anchor),
            Combinator::Child => self.parent[subject] == Some(anchor),
            Combinator::NextSibling => self.previous[subject] == Some(anchor),
            Combinator::SubsequentSibling => self
                .previous_siblings(subject)
                .any(|sibling| sibling == anchor),
            Combinator::Column => false,
        }
    }

    fn matches_has(&self, list: &SelectorList, anchor: usize) -> bool {
        list.selectors.iter().any(|selector| {
            let Some(last) = selector.compounds.len().checked_sub(1) else {
                return false;
            };
            let mut check = |candidate| self.match_from(selector, last, candidate, Some(anchor));
            match selector
                .leading_combinator
                .unwrap_or(Combinator::Descendant)
            {
                Combinator::Descendant | Combinator::Child => {
                    self.any_descendant(anchor, &mut check)
                }
                Combinator::NextSibling | Combinator::SubsequentSibling => self
                    .next_siblings(anchor)
                    .any(|sibling| check(sibling) || self.any_descendant(sibling, &mut check)),
                Combinator::Column => false,
            }
        })
    }

    fn matches_compound(&self, compound: &CompoundSelector, index: usize) -> bool {
        let Some(element) = self.element(index) else {
            return false;
        };
        if let Some(type_selector) = &compound.type_selector
            && !matches_type(type_selector, element)
        {
            return false;
        }
        compound.simples.iter().all(|simple| match simple {
            SimpleSelector::Id(id) => attribute(element, "id") == Some(id.as_str()),
            SimpleSelector::Class(class) => attribute(element, "class")
                .is_some_and(|classes| classes.split_ascii_whitespace().any(|item| item == class)),
            SimpleSelector::Attribute(selector) => matches_attribute(selector, element),
            SimpleSelector::Pseudo(pseudo) => self.matches_pseudo(pseudo, index, element),
            // Outside a nested rule, `&` behaves like `:scope`, which is the
            // root element for document stylesheets.
            SimpleSelector::Nesting => self.is_root[index],
        })
    }

    fn matches_pseudo(&self, pseudo: &PseudoSelector, index: usize, element: &ElementData) -> bool {
        if pseudo.element {
            return false;
        }
        let name = lowercase(&pseudo.name);
        let nested = pseudo.nested_selectors.as_deref();
        let tag = element.get_name().as_str();
        match name.as_ref() {
            "root" | "scope" => self.is_root[index],
            "is" | "where" | "matches" | "-webkit-any" | "-moz-any" => {
                nested.is_some_and(|list| self.matches_any(list, index))
            }
            "not" => nested.is_some_and(|list| !self.matches_any(list, index)),
            "has" => nested.is_some_and(|list| self.matches_has(list, index)),
            "first-child" => self.previous[index].is_none(),
            "last-child" => self.next[index].is_none(),
            "only-child" => self.previous[index].is_none() && self.next[index].is_none(),
            "first-of-type" => !self
                .previous_siblings(index)
                .any(|s| self.same_type(s, element)),
            "last-of-type" => !self
                .next_siblings(index)
                .any(|s| self.same_type(s, element)),
            "only-of-type" => {
                !self
                    .previous_siblings(index)
                    .any(|s| self.same_type(s, element))
                    && !self
                        .next_siblings(index)
                        .any(|s| self.same_type(s, element))
            }
            "nth-child" | "nth-last-child" | "nth-of-type" | "nth-last-of-type" => {
                self.matches_nth(&name, pseudo, index, element)
            }
            "empty" => self.dom.nodes[index].get_children().iter().all(|child| {
                match self.dom.nodes[child.get()].get_kind() {
                    NodeKind::Element(_) => false,
                    NodeKind::Text(text) => text.is_empty(),
                    _ => true,
                }
            }),
            "link" | "any-link" | "-webkit-any-link" => {
                matches!(tag, "a" | "area") && attribute(element, "href").is_some()
            }
            "defined" => !tag.contains('-'),
            "enabled" => FORM_CONTROLS.contains(&tag) && attribute(element, "disabled").is_none(),
            "disabled" => FORM_CONTROLS.contains(&tag) && attribute(element, "disabled").is_some(),
            "checked" => match tag {
                "input" => {
                    attribute(element, "checked").is_some()
                        && attribute(element, "type").is_some_and(|kind| {
                            kind.eq_ignore_ascii_case("checkbox")
                                || kind.eq_ignore_ascii_case("radio")
                        })
                }
                "option" => attribute(element, "selected").is_some(),
                _ => false,
            },
            "required" => {
                matches!(tag, "input" | "select" | "textarea")
                    && attribute(element, "required").is_some()
            }
            "optional" => {
                matches!(tag, "input" | "select" | "textarea")
                    && attribute(element, "required").is_none()
            }
            "read-write" => is_read_write(element),
            "read-only" => !is_read_write(element),
            "placeholder-shown" => {
                matches!(tag, "input" | "textarea")
                    && attribute(element, "placeholder").is_some_and(|text| !text.is_empty())
                    && attribute(element, "value").is_none_or(str::is_empty)
            }
            "open" => matches!(tag, "details" | "dialog") && attribute(element, "open").is_some(),
            "lang" => pseudo.argument.as_deref().is_some_and(|wanted| {
                let wanted = wanted
                    .trim_matches(|c| c == '"' || c == '\'')
                    .to_ascii_lowercase();
                self.inherited_attribute(index, "lang").is_some_and(|lang| {
                    let lang = lang.to_ascii_lowercase();
                    lang == wanted || lang.starts_with(&format!("{wanted}-"))
                })
            }),
            "dir" => pseudo.argument.as_deref().is_some_and(|wanted| {
                let direction = self
                    .inherited_attribute(index, "dir")
                    .filter(|dir| {
                        dir.eq_ignore_ascii_case("rtl") || dir.eq_ignore_ascii_case("ltr")
                    })
                    .unwrap_or("ltr");
                direction.eq_ignore_ascii_case(wanted.trim())
            }),
            other => {
                log::trace!("unsupported pseudo-class `:{other}` never matches");
                false
            }
        }
    }

    /// The nearest value of attribute `name` on the element or an ancestor.
    fn inherited_attribute(&self, index: usize, name: &str) -> Option<&'d str> {
        std::iter::once(index)
            .chain(self.ancestors(index))
            .find_map(|current| self.element(current).and_then(|e| attribute(e, name)))
    }

    fn same_type(&self, index: usize, element: &ElementData) -> bool {
        self.element(index).is_some_and(|other| {
            other.get_name() == element.get_name()
                && other.get_namespace() == element.get_namespace()
        })
    }

    fn matches_nth(
        &self,
        name: &str,
        pseudo: &PseudoSelector,
        index: usize,
        element: &ElementData,
    ) -> bool {
        let Some(argument) = pseudo.argument.as_deref() else {
            return false;
        };
        let formula = split_of_keyword(argument);
        let Some((step, offset)) = parse_an_plus_b(formula) else {
            log::trace!("invalid An+B `{formula}` never matches");
            return false;
        };
        let of_selectors = pseudo.nested_selectors.as_deref();
        if let Some(list) = of_selectors
            && !self.matches_any(list, index)
        {
            return false;
        }
        let counts = |sibling: &usize| match name {
            "nth-of-type" | "nth-last-of-type" => self.same_type(*sibling, element),
            _ => of_selectors.is_none_or(|list| self.matches_any(list, *sibling)),
        };
        let before = match name {
            "nth-child" | "nth-of-type" => self.previous_siblings(index).filter(counts).count(),
            _ => self.next_siblings(index).filter(counts).count(),
        };
        nth_matches(step, offset, before as i64 + 1)
    }
}

fn matches_type(selector: &TypeSelector, element: &ElementData) -> bool {
    match selector.namespace.as_deref() {
        None | Some("*") => {}
        // Every DOM element has a namespace, and `@namespace` prefixes are
        // not supported, so explicit namespaces never match.
        Some(_) => return false,
    }
    selector.name == "*"
        || match element.get_namespace() {
            Namespace::Html => selector.name.eq_ignore_ascii_case(element.get_name()),
            _ => selector.name == *element.get_name(),
        }
}

fn matches_attribute(selector: &AttributeSelector, element: &ElementData) -> bool {
    if !matches!(selector.namespace.as_deref(), None | Some("*") | Some("")) {
        return false;
    }
    let html = *element.get_namespace() == Namespace::Html;
    // HTML attribute names are stored lowercase, so compare lowercase names.
    let name = if html {
        lowercase(&selector.name)
    } else {
        Cow::Borrowed(selector.name.as_str())
    };
    let Some(actual) = element
        .get_attributes()
        .iter()
        .find(|attribute| *attribute.get_name() == *name)
        .map(|attribute| attribute.get_value().as_str())
    else {
        return false;
    };
    let (Some(operator), Some(expected)) = (selector.operator, selector.value.as_deref()) else {
        return true;
    };
    let insensitive = match selector.modifier {
        Some('i') => true,
        Some(_) => false,
        None => {
            html && CASE_INSENSITIVE_HTML_ATTRIBUTES
                .binary_search(&name.as_ref())
                .is_ok()
        }
    };
    let (actual, expected) = if insensitive {
        (
            Cow::Owned(actual.to_ascii_lowercase()),
            Cow::Owned(expected.to_ascii_lowercase()),
        )
    } else {
        (Cow::Borrowed(actual), Cow::Borrowed(expected))
    };
    let (actual, expected) = (actual.as_ref(), expected.as_ref());
    match operator {
        AttributeOperator::Equals => actual == expected,
        AttributeOperator::Includes => {
            !expected.is_empty()
                && !expected.contains(char::is_whitespace)
                && actual.split_ascii_whitespace().any(|word| word == expected)
        }
        AttributeOperator::DashMatch => {
            actual == expected
                || actual
                    .strip_prefix(expected)
                    .is_some_and(|rest| rest.starts_with('-'))
        }
        AttributeOperator::Prefix => !expected.is_empty() && actual.starts_with(expected),
        AttributeOperator::Suffix => !expected.is_empty() && actual.ends_with(expected),
        AttributeOperator::Substring => !expected.is_empty() && actual.contains(expected),
    }
}

fn is_read_write(element: &ElementData) -> bool {
    let editable =
        attribute(element, "readonly").is_none() && attribute(element, "disabled").is_none();
    match element.get_name().as_str() {
        "textarea" => editable,
        "input" => {
            editable
                && attribute(element, "type").is_none_or(|kind| {
                    !matches!(
                        kind.to_ascii_lowercase().as_str(),
                        "hidden"
                            | "range"
                            | "color"
                            | "checkbox"
                            | "radio"
                            | "file"
                            | "submit"
                            | "image"
                            | "reset"
                            | "button"
                    )
                })
        }
        _ => attribute(element, "contenteditable")
            .is_some_and(|value| !value.eq_ignore_ascii_case("false")),
    }
}

/// Returns the An+B part of an `:nth-*()` argument, before any `of S`.
fn split_of_keyword(argument: &str) -> &str {
    let lower = argument.to_ascii_lowercase();
    let mut search = 0;
    while let Some(found) = lower[search..].find("of") {
        let start = search + found;
        let end = start + 2;
        let before_ok = start > 0 && lower.as_bytes()[start - 1].is_ascii_whitespace();
        let after_ok = lower
            .as_bytes()
            .get(end)
            .is_some_and(u8::is_ascii_whitespace);
        if before_ok && after_ok {
            return argument[..start].trim();
        }
        search = end;
    }
    argument.trim()
}

/// Parses the An+B microsyntax (CSS Syntax 3 §6) into `(A, B)`.
///
/// Whitespace is accepted anywhere, which is slightly more lenient than the
/// grammar. Returns `None` for text that is not An+B.
pub(crate) fn parse_an_plus_b(text: &str) -> Option<(i64, i64)> {
    let compact: String = text
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    match compact.as_str() {
        "odd" => return Some((2, 1)),
        "even" => return Some((2, 0)),
        "" => return None,
        _ => {}
    }
    let Some(n) = compact.find('n') else {
        return parse_signed(&compact).map(|offset| (0, offset));
    };
    let step = match &compact[..n] {
        "" | "+" => 1,
        "-" => -1,
        digits => parse_signed(digits)?,
    };
    let rest = &compact[n + 1..];
    let offset = if rest.is_empty() {
        0
    } else if rest.starts_with('+') || rest.starts_with('-') {
        parse_signed(rest)?
    } else {
        return None;
    };
    Some((step, offset))
}

fn parse_signed(text: &str) -> Option<i64> {
    let digits = text.strip_prefix('+').unwrap_or(text);
    let unsigned = digits.strip_prefix('-').unwrap_or(digits);
    if unsigned.is_empty() || !unsigned.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Whether 1-based `position` equals `step * n + offset` for some `n >= 0`.
fn nth_matches(step: i64, offset: i64, position: i64) -> bool {
    if step == 0 {
        return position == offset;
    }
    let difference = position - offset;
    difference % step == 0 && difference / step >= 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selector::parse_selector_list;

    fn matching_ids(html: &str, selector: &str) -> Vec<String> {
        let dom = html::parse_raw_html(html.into());
        let tree = ElementTree::new(&dom);
        let list = parse_selector_list(selector).expect("valid selector");
        (0..dom.nodes.len())
            .filter(|index| tree.matches_any(&list, *index))
            .filter_map(|index| tree.element(index).and_then(|e| attribute(e, "id")))
            .map(str::to_owned)
            .collect()
    }

    const LIST: &str = "<ul id='list' class='menu'>\
        <li id='a' class='item'>A</li>\
        <li id='b' class='item active'><a id='link' href='/x'>B</a></li>\
        <li id='c'>C</li>\
        <li id='d' class='item'></li>\
        </ul>";

    #[test]
    fn matches_combinators() {
        assert_eq!(matching_ids(LIST, "ul > li.active"), ["b"]);
        assert_eq!(matching_ids(LIST, ".menu a"), ["link"]);
        assert_eq!(matching_ids(LIST, "#a + li"), ["b"]);
        assert_eq!(matching_ids(LIST, "#b ~ li"), ["c", "d"]);
        assert!(matching_ids(LIST, "li > ul").is_empty());
    }

    #[test]
    fn matches_attributes() {
        assert_eq!(matching_ids(LIST, "[href^='/']"), ["link"]);
        assert_eq!(matching_ids(LIST, "[class~=active]"), ["b"]);
        assert_eq!(matching_ids(LIST, "[CLASS*=ACT i]"), ["b"]);
        assert!(matching_ids(LIST, "[class*=ACT]").is_empty());
        assert_eq!(
            matching_ids("<p id='p' lang='en-US'></p>", "[lang|=en]"),
            ["p"]
        );
        assert_eq!(
            matching_ids("<input id='i' type='TEXT'>", "[type=text]"),
            ["i"]
        );
    }

    #[test]
    fn matches_structural_pseudo_classes() {
        assert_eq!(matching_ids(LIST, "li:first-child"), ["a"]);
        assert_eq!(matching_ids(LIST, "li:last-child"), ["d"]);
        assert_eq!(matching_ids(LIST, "li:nth-child(odd)"), ["a", "c"]);
        assert_eq!(matching_ids(LIST, "li:nth-last-child(-n + 2)"), ["c", "d"]);
        assert_eq!(matching_ids(LIST, "li:nth-child(2 of .item)"), ["b"]);
        assert_eq!(matching_ids(LIST, "li:empty"), ["d"]);
        assert_eq!(matching_ids(LIST, "a:only-of-type"), ["link"]);
        // `parse_raw_html` adds no implicit `<html>`, so the list is the root.
        assert_eq!(matching_ids(LIST, ":root"), ["list"]);
    }

    #[test]
    fn matches_logical_pseudo_classes_and_has() {
        assert_eq!(matching_ids(LIST, "li:not(.item)"), ["c"]);
        assert_eq!(matching_ids(LIST, ":is(#a, #c)"), ["a", "c"]);
        assert_eq!(matching_ids(LIST, "li:has(> a[href])"), ["b"]);
        assert_eq!(matching_ids(LIST, "ul:has(.active)"), ["list"]);
        assert_eq!(matching_ids(LIST, "li:has(+ #c)"), ["b"]);
        assert_eq!(matching_ids(LIST, "a:link, a:any-link"), ["link"]);
    }

    #[test]
    fn dynamic_pseudo_classes_and_pseudo_elements_never_match() {
        assert!(matching_ids(LIST, "a:hover, a:visited, li::before, li:after").is_empty());
        let list = parse_selector_list("a:hover").unwrap();
        assert!(compound_never_matches(&list.selectors[0].compounds[0]));
    }

    #[test]
    fn case_insensitive_attribute_list_is_sorted() {
        assert!(CASE_INSENSITIVE_HTML_ATTRIBUTES.is_sorted());
    }

    #[test]
    fn parses_an_plus_b() {
        assert_eq!(parse_an_plus_b("2n+1"), Some((2, 1)));
        assert_eq!(parse_an_plus_b(" -n + 3 "), Some((-1, 3)));
        assert_eq!(parse_an_plus_b("+5"), Some((0, 5)));
        assert_eq!(parse_an_plus_b("n"), Some((1, 0)));
        assert_eq!(parse_an_plus_b("EVEN"), Some((2, 0)));
        assert_eq!(parse_an_plus_b("2n+"), None);
        assert_eq!(parse_an_plus_b("foo"), None);
        assert!(nth_matches(-1, 3, 3) && nth_matches(-1, 3, 1) && !nth_matches(-1, 3, 4));
    }
}
