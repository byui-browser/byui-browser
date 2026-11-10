//! Cascade and inheritance.
//!
//! Selector matching lives in [`crate::matching`]; the rule hash that limits
//! which rules an element is tested against lives in [`crate::rule_index`].

use crate::computed::{ComputedStyles, computed_style};
use crate::matching::{ElementTree, attribute};
use crate::parser::parse_declarations;
use crate::rule_index::{CompiledDeclaration, CompiledSheet, RuleIndex, compile_declarations};
use crate::stylesheet::{Declaration, Dom, Specificity, Stylesheet};
use crate::user_agent::{user_agent_compiled, user_agent_stylesheet};
use crate::values;
use html::{NodeID, NodeKind};
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap};

/// Cascade precedence for a declaration, compared lexicographically.
///
/// `level` orders origins and importance per CSS Cascade 4 §6.1: normal
/// user-agent < normal author < normal inline style < important author <
/// important inline style < important user-agent. The remaining fields are
/// specificity and then source order: stylesheet, rule, and declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct CascadePriority {
    level: u8,
    specificity: Specificity,
    sheet: usize,
    rule: usize,
    declaration: usize,
}

const USER_AGENT_NORMAL: u8 = 1;
const AUTHOR_NORMAL: u8 = 2;
const INLINE_NORMAL: u8 = 3;
const AUTHOR_IMPORTANT: u8 = 4;
const INLINE_IMPORTANT: u8 = 5;
const USER_AGENT_IMPORTANT: u8 = 6;

/// Counters from one cascade run, used for logging and regression tests.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CascadeStats {
    /// Elements styled.
    pub(crate) elements: usize,
    /// Selectors that matching rules could contribute, across all sheets.
    pub(crate) indexed_selectors: usize,
    /// Full selector matches attempted on candidate rules.
    pub(crate) selectors_checked: usize,
    /// Selector matches that succeeded.
    pub(crate) selectors_matched: usize,
    /// Longhand declarations that competed in the cascade.
    pub(crate) declarations_applied: usize,
}

/// Applies the bundled user-agent stylesheet, author stylesheets, inline
/// styles, and inheritance to an HTML document.
///
/// Declarations compete by origin and importance (`!important` is honored and
/// stripped from the value), then specificity, then source order. Shorthands
/// are expanded to longhands before they compete; see [`crate::shorthands::expand`].
/// The CSS-wide keywords `inherit`, `initial`, and `unset` are resolved;
/// `revert` and `revert-layer` are treated as `unset`.
///
/// Inherited properties receive the parent's computed value. Every element
/// gets a computed `font-size` in absolute CSS pixels (CSS Fonts 4 §2.5), so
/// descendants inherit an absolute size rather than re-resolving `em`. Other
/// values are passed through as specified text.
///
/// Selector support is described in [`crate::selector`] (syntax) and the
/// matcher: combinators, attribute selectors, and structural and logical
/// pseudo-classes match; dynamic pseudo-classes such as `:hover` and all
/// pseudo-elements never match. Rules inside at-rules such as `@media` are
/// not applied yet.
///
/// Cost: each stylesheet is compiled once per call, and each element is only
/// tested against rules filed under its ID, classes, attribute names, and tag
/// (plus rules with no such key), not against every rule.
pub fn style_document(dom: &Dom, stylesheets: &[Stylesheet]) -> ComputedStyles {
    style_document_with_stats(dom, stylesheets).0
}

pub(crate) fn style_document_with_stats(
    dom: &Dom,
    stylesheets: &[Stylesheet],
) -> (ComputedStyles, CascadeStats) {
    log::debug!(
        "styling document: {} nodes, {} author stylesheets",
        dom.nodes.len(),
        stylesheets.len()
    );
    if log::log_enabled!(log::Level::Debug) {
        let ignored_at_rules: usize = stylesheets.iter().map(|sheet| sheet.at_rules.len()).sum();
        if ignored_at_rules > 0 {
            // Debug, not warn: the shell caches the rendered page but restyles
            // it on window resize and on page clicks (hit tests), so a warning
            // here would repeat on each of those.
            log::debug!(
                "{ignored_at_rules} at-rules in author stylesheets are not applied by the cascade"
            );
        }
    }

    let author_sheets: Vec<CompiledSheet> =
        stylesheets.iter().map(CompiledSheet::compile).collect();
    let mut index = RuleIndex::default();
    index.add_sheet(
        user_agent_compiled(),
        0,
        USER_AGENT_NORMAL,
        USER_AGENT_IMPORTANT,
    );
    for (position, sheet) in author_sheets.iter().enumerate() {
        index.add_sheet(sheet, position + 1, AUTHOR_NORMAL, AUTHOR_IMPORTANT);
    }
    let tree = ElementTree::new(dom);
    let mut stats = CascadeStats {
        indexed_selectors: index.rules.len(),
        ..CascadeStats::default()
    };

    let mut computed: Vec<(NodeID, Vec<Declaration>)> = Vec::new();
    let mut root_font_size = None;
    let mut candidates = Vec::new();
    let initial_color = initial_color_value();
    for (node_index, node) in dom.nodes.iter().enumerate() {
        let NodeKind::Element(element) = node.get_kind() else {
            continue;
        };
        stats.elements += 1;
        let parent = node
            .get_parent()
            .and_then(|parent| computed_style(&computed, parent));
        let parent_value = |property: &str| {
            parent.and_then(|declarations| {
                declarations
                    .iter()
                    .find(|declaration| declaration.property == property)
                    .map(|declaration| declaration.value.as_str())
            })
        };

        // Inline declarations must outlive `cascaded`, which borrows them.
        let inline: Vec<CompiledDeclaration> = match attribute(element, "style") {
            Some(style) => {
                let mut inline_errors = Stylesheet::default();
                compile_declarations(&parse_declarations(style, &mut inline_errors, 0))
            }
            None => Vec::new(),
        };

        let mut cascaded = HashMap::<&str, (CascadePriority, &str)>::new();
        index.candidates(element, &mut candidates);
        for &candidate in &candidates {
            let indexed = &index.rules[candidate as usize];
            stats.selectors_checked += 1;
            if !tree.matches(&indexed.rule.selector, node_index) {
                continue;
            }
            stats.selectors_matched += 1;
            log::trace!(
                "rule {} of sheet {} matches node {node_index}",
                indexed.rule.rule_index,
                indexed.sheet_index
            );
            for (position, declaration) in indexed.rule.declarations.iter().enumerate() {
                let level = if declaration.important {
                    indexed.important_level
                } else {
                    indexed.normal_level
                };
                stats.declarations_applied += 1;
                apply(
                    &mut cascaded,
                    declaration,
                    CascadePriority {
                        level,
                        specificity: indexed.rule.specificity,
                        sheet: indexed.sheet_index,
                        rule: indexed.rule.rule_index,
                        declaration: position,
                    },
                );
            }
        }
        for (position, declaration) in inline.iter().enumerate() {
            let level = if declaration.important {
                INLINE_IMPORTANT
            } else {
                INLINE_NORMAL
            };
            stats.declarations_applied += 1;
            apply(
                &mut cascaded,
                declaration,
                CascadePriority {
                    level,
                    specificity: Specificity(0, 0, 0),
                    sheet: 0,
                    rule: 0,
                    declaration: position,
                },
            );
        }

        let mut properties = BTreeMap::<String, String>::new();
        if let Some(parent) = parent {
            for declaration in parent {
                if is_inherited_property(&declaration.property) {
                    properties.insert(declaration.property.clone(), declaration.value.clone());
                }
            }
        }
        properties
            .entry("color".to_owned())
            .or_insert_with(|| initial_color.clone());
        for (property, (_, value)) in cascaded {
            let keyword = value.trim();
            let inherit = if keyword.eq_ignore_ascii_case("inherit") {
                true
            } else if ["unset", "revert", "revert-layer"]
                .iter()
                .any(|word| keyword.eq_ignore_ascii_case(word))
            {
                is_inherited_property(property)
            } else if keyword.eq_ignore_ascii_case("initial") {
                false
            } else {
                properties.insert(property.to_owned(), value.to_owned());
                continue;
            };
            match parent_value(property).filter(|_| inherit) {
                Some(inherited) => {
                    properties.insert(property.to_owned(), inherited.to_owned());
                }
                None if property == "color" => {
                    properties.insert(property.to_owned(), initial_color.clone());
                }
                None => {
                    properties.remove(property);
                }
            }
        }

        let parent_font_size = parent_value("font-size")
            .and_then(|value| {
                values::compute_font_size(value, values::MEDIUM_FONT_SIZE, values::MEDIUM_FONT_SIZE)
            })
            .unwrap_or(values::MEDIUM_FONT_SIZE);
        let root_size = root_font_size.unwrap_or(values::MEDIUM_FONT_SIZE);
        let font_size = properties
            .get("font-size")
            .and_then(|value| values::compute_font_size(value, parent_font_size, root_size))
            .unwrap_or_else(|| {
                log::trace!("font-size not computable for node {node_index}; using parent size");
                parent_font_size
            });
        properties.insert("font-size".to_owned(), values::format_px(font_size));
        if root_font_size.is_none() {
            root_font_size = Some(font_size);
        }
        computed.push((
            NodeID::new(node_index),
            properties
                .into_iter()
                .map(|(property, value)| Declaration { property, value })
                .collect(),
        ));
    }
    log::debug!(
        "styled document: {} elements, {} indexed selectors, {} selector checks ({} matched), \
         {} longhand declarations considered",
        stats.elements,
        stats.indexed_selectors,
        stats.selectors_checked,
        stats.selectors_matched,
        stats.declarations_applied
    );
    (ComputedStyles { styles: computed }, stats)
}

/// Records `declaration` for its property if it beats the current winner.
///
/// Ties keep the later declaration, although the priority's source-order
/// fields make ties between different declarations impossible.
fn apply<'d>(
    cascaded: &mut HashMap<&'d str, (CascadePriority, &'d str)>,
    declaration: &'d CompiledDeclaration,
    priority: CascadePriority,
) {
    log::trace!(
        "declaration `{}: {}` level={}",
        declaration.property,
        declaration.value,
        priority.level
    );
    match cascaded.entry(declaration.property.as_str()) {
        Entry::Occupied(mut current) => {
            if priority >= current.get().0 {
                current.insert((priority, declaration.value.as_str()));
            }
        }
        Entry::Vacant(slot) => {
            slot.insert((priority, declaration.value.as_str()));
        }
    }
}

/// The initial `color`, taken from the user-agent `html` rule.
fn initial_color_value() -> String {
    user_agent_stylesheet()
        .rules
        .iter()
        .find(|rule| rule.selector.0 == "html")
        .and_then(|rule| {
            rule.declarations
                .iter()
                .find(|item| item.property == "color")
        })
        .expect("user-agent stylesheet must define html color")
        .value
        .clone()
}

fn is_inherited_property(property: &str) -> bool {
    matches!(
        property,
        "color"
            | "font-family"
            | "font-size"
            | "font-style"
            | "font-weight"
            | "line-height"
            | "text-align"
            | "text-transform"
            | "visibility"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ComputedStyles, Declaration, parse_color, parse_stylesheet};
    use html::HTMLDocument;

    #[test]
    fn cascades_matches_and_inlines() {
        let document = html::parse_raw_html(
            "<div><p id='target' class='intro' style='color: black'>Hello</p></div>".into(),
        );
        let stylesheet =
            parse_stylesheet("p { color: green; } .intro { color: red; font-size: 20px; }");
        let styles = style_document(&document, &[stylesheet]);
        let paragraph = document
            .get_element_by_id("target")
            .expect("paragraph node id");
        let declarations = styles
            .styles
            .iter()
            .find(|(id, _)| id.get() == paragraph.get())
            .expect("computed paragraph");
        assert!(declarations.1.contains(&Declaration {
            property: "color".into(),
            value: "black".into()
        }));
        assert!(declarations.1.contains(&Declaration {
            property: "font-size".into(),
            value: "20px".into()
        }));
    }

    #[test]
    fn bundled_defaults_cascade_before_author_styles_and_inherit() {
        let document = html::parse_raw_html(
            "<html id='root'><body><main id='content'>Hello</main></body></html>".into(),
        );
        let html = document.get_element_by_id("root").expect("html node");
        let main = document.get_element_by_id("content").expect("main node");
        let defaults = style_document(&document, &[]);
        assert_eq!(defaults.color_for(html, "background-color"), None);
        assert_eq!(defaults.color_for(main, "color"), parse_color("black"));

        let author = parse_stylesheet("html { color: red; background-color: #123456; }");
        let styled = style_document(&document, &[author]);
        assert_eq!(
            styled.color_for(html, "background-color"),
            parse_color("#123456")
        );
        assert_eq!(styled.color_for(main, "color"), parse_color("red"));
    }

    fn computed(
        styles: &ComputedStyles,
        document: &HTMLDocument,
        id: &str,
        property: &str,
    ) -> Option<String> {
        let node = document.get_element_by_id(id).expect("element with id");
        styles.value_for(node, property).map(str::to_owned)
    }

    #[test]
    fn important_declarations_beat_later_and_more_specific_ones() {
        let document = html::parse_raw_html("<p id='target' class='a'>Hi</p>".into());
        let sheet = parse_stylesheet(
            "p { color: green !important; } .a { color: red; } #target { color: blue; }",
        );
        let styles = style_document(&document, &[sheet]);
        assert_eq!(
            computed(&styles, &document, "target", "color").as_deref(),
            Some("green")
        );
    }

    #[test]
    fn author_shorthand_overrides_user_agent_longhand() {
        let document = html::parse_raw_html("<p id='target'>Hi</p>".into());
        let styles = style_document(&document, &[parse_stylesheet("p { margin: 0 }")]);
        assert_eq!(
            computed(&styles, &document, "target", "margin-top").as_deref(),
            Some("0")
        );
        assert_eq!(
            computed(&styles, &document, "target", "margin-left").as_deref(),
            Some("0")
        );
    }

    #[test]
    fn font_size_inherits_as_an_absolute_length() {
        let document = html::parse_raw_html(
            "<div id='outer' style='font-size: 2em'><span id='inner'>Hi</span><b id='rel' style='font-size: 50%'>x</b></div>"
                .into(),
        );
        let styles = style_document(&document, &[]);
        assert_eq!(
            computed(&styles, &document, "outer", "font-size").as_deref(),
            Some("32px")
        );
        assert_eq!(
            computed(&styles, &document, "inner", "font-size").as_deref(),
            Some("32px")
        );
        assert_eq!(
            computed(&styles, &document, "rel", "font-size").as_deref(),
            Some("16px")
        );
    }

    #[test]
    fn css_wide_keywords_resolve_against_the_parent() {
        let document = html::parse_raw_html(
            "<div id='parent' style='color: red; margin-left: 5px'><p id='child' style='color: initial; margin-left: inherit'>Hi</p></div>"
                .into(),
        );
        let styles = style_document(&document, &[]);
        assert_eq!(
            computed(&styles, &document, "child", "color").as_deref(),
            Some("black")
        );
        assert_eq!(
            computed(&styles, &document, "child", "margin-left").as_deref(),
            Some("5px")
        );
    }

    #[test]
    fn source_order_breaks_specificity_ties_across_index_buckets() {
        let document = html::parse_raw_html("<p id='target' class='a' data-x='1'>Hi</p>".into());
        for (css, expected) in [
            ("[data-x] { color: red } .a { color: blue }", "blue"),
            (".a { color: blue } [data-x] { color: red }", "red"),
        ] {
            let styles = style_document(&document, &[parse_stylesheet(css)]);
            assert_eq!(
                computed(&styles, &document, "target", "color").as_deref(),
                Some(expected),
                "{css}"
            );
        }
    }

    #[test]
    fn later_stylesheets_win_ties_and_declarations_keep_their_order() {
        let document = html::parse_raw_html("<p id='target' class='a'>Hi</p>".into());
        let first = parse_stylesheet(".a { color: red }");
        let second = parse_stylesheet(".a { color: blue; margin: 0; margin-top: 3px }");
        let styles = style_document(&document, &[first, second]);
        assert_eq!(
            computed(&styles, &document, "target", "color").as_deref(),
            Some("blue")
        );
        assert_eq!(
            computed(&styles, &document, "target", "margin-top").as_deref(),
            Some("3px")
        );
        assert_eq!(
            computed(&styles, &document, "target", "margin-left").as_deref(),
            Some("0")
        );
    }

    #[test]
    fn combinators_attributes_and_pseudo_classes_take_part_in_the_cascade() {
        let document = html::parse_raw_html(
            "<div class='card'><p id='first'>a</p><p id='second' hidden>b</p>\
             <a id='link' href='/x'>c</a><span id='plain'>d</span></div>"
                .into(),
        );
        let sheet = parse_stylesheet(
            ".card > p:first-child { color: red } \
             p + p { color: blue } \
             div :not(p, a) { color: green } \
             span:hover { color: orange } \
             span::before { display: block }",
        );
        let styles = style_document(&document, &[sheet]);
        let value = |id, property| computed(&styles, &document, id, property);
        assert_eq!(value("first", "color").as_deref(), Some("red"));
        assert_eq!(value("second", "color").as_deref(), Some("blue"));
        assert_eq!(value("plain", "color").as_deref(), Some("green"));
        // `span::before` styles a pseudo-element, never the span itself.
        assert_eq!(value("plain", "display"), None);
        // User-agent rules that rely on attribute and pseudo-class selectors.
        assert_eq!(value("second", "display").as_deref(), Some("none"));
        assert_eq!(value("link", "color").as_deref(), Some("#0000ee"));
    }

    #[test]
    fn elements_are_only_tested_against_rules_that_could_match_them() {
        let mut css = String::new();
        let mut html = String::from("<div>");
        for index in 0..500 {
            css.push_str(&format!(
                ".c{index} {{ color: red }} #i{index} {{ color: blue }}\n"
            ));
            html.push_str(&format!("<p class='c{index}' id='i{index}'>x</p>"));
        }
        html.push_str("</div>");
        let document = html::parse_raw_html(html);
        let (styles, stats) = style_document_with_stats(&document, &[parse_stylesheet(&css)]);
        assert_eq!(
            computed(&styles, &document, "i42", "color").as_deref(),
            Some("blue")
        );
        // Testing every element against every rule would be about
        // 500 elements x 1,000 author rules. The rule hash keeps the work to
        // each element's own two rules plus a handful of user-agent rules.
        assert!(stats.elements > 500);
        assert!(
            stats.selectors_checked < stats.elements * 10,
            "too many selector checks: {stats:?}"
        );
    }
}
