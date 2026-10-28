//! Cascade, inheritance, and selector matching.

use crate::computed::{ComputedStyles, computed_style};
use crate::parser::parse_declarations;
use crate::shorthands;
use crate::specificity::specificity;
use crate::stylesheet::{Declaration, Dom, Specificity, Stylesheet};
use crate::user_agent::user_agent_stylesheet;
use crate::values;
use html::{HTMLDocument, NodeID, NodeKind};
use std::collections::BTreeMap;

/// Cascade precedence for a declaration, compared lexicographically.
///
/// `level` orders origins and importance per CSS Cascade 4 §6.1: normal
/// user-agent < normal author < normal inline style < important author <
/// important inline style < important user-agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct CascadePriority {
    level: u8,
    specificity: Specificity,
    order: usize,
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
pub fn style_document(dom: &Dom, stylesheets: &[Stylesheet]) -> ComputedStyles {
    let mut computed: Vec<(NodeID, Vec<Declaration>)> = Vec::new();
    let mut root_font_size = None;
    for (index, node) in dom.nodes.iter().enumerate() {
        let NodeKind::Element(element) = &node.kind else {
            continue;
        };
        let parent = node
            .parent
            .and_then(|parent| computed_style(&computed, parent));
        let parent_value = |property: &str| {
            parent.and_then(|declarations| {
                declarations
                    .iter()
                    .find(|declaration| declaration.property == property)
                    .map(|declaration| declaration.value.clone())
            })
        };

        let mut cascaded = BTreeMap::<String, (CascadePriority, String)>::new();
        let mut apply = |declaration: &Declaration,
                         normal_level: u8,
                         important_level: u8,
                         specificity,
                         order| {
            let (value, important) = split_important(&declaration.value);
            let level = if important {
                important_level
            } else {
                normal_level
            };
            let priority = CascadePriority {
                level,
                specificity,
                order,
            };
            let expanded = shorthands::expand(&Declaration {
                property: declaration.property.clone(),
                value: value.to_owned(),
            });
            for longhand in expanded {
                let wins = cascaded
                    .get(&longhand.property)
                    .is_none_or(|(current, _)| priority >= *current);
                if wins {
                    cascaded.insert(longhand.property, (priority, longhand.value));
                }
            }
        };
        for (sheet_index, (normal_level, important_level, sheet)) in
            std::iter::once((1, 6, user_agent_stylesheet()))
                .chain(stylesheets.iter().map(|sheet| (2, 4, sheet)))
                .enumerate()
        {
            for (rule_index, rule) in sheet.rules.iter().enumerate() {
                if !matches_selector(dom, NodeID::new(index), &rule.selector.0) {
                    continue;
                }
                let order = sheet_index
                    .saturating_mul(1_000_000)
                    .saturating_add(rule_index);
                let selector_specificity = specificity(&rule.selector);
                for declaration in &rule.declarations {
                    apply(
                        declaration,
                        normal_level,
                        important_level,
                        selector_specificity,
                        order,
                    );
                }
            }
        }
        if let Some(style) = attribute_value(element, "style") {
            let mut inline_errors = Stylesheet::default();
            for (position, declaration) in parse_declarations(style, &mut inline_errors, 0)
                .iter()
                .enumerate()
            {
                apply(declaration, 3, 5, Specificity(0, 0, 0), position);
            }
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
            .or_insert_with(initial_color_value);
        for (property, (_, value)) in cascaded {
            let keyword = value.trim().to_ascii_lowercase();
            let inherit = match keyword.as_str() {
                "inherit" => true,
                "unset" | "revert" | "revert-layer" => is_inherited_property(&property),
                "initial" => false,
                _ => {
                    properties.insert(property, value);
                    continue;
                }
            };
            match parent_value(&property).filter(|_| inherit) {
                Some(inherited) => {
                    properties.insert(property, inherited);
                }
                None if property == "color" => {
                    properties.insert(property, initial_color_value());
                }
                None => {
                    properties.remove(&property);
                }
            }
        }

        let parent_font_size = parent_value("font-size")
            .and_then(|value| {
                values::compute_font_size(
                    &value,
                    values::MEDIUM_FONT_SIZE,
                    values::MEDIUM_FONT_SIZE,
                )
            })
            .unwrap_or(values::MEDIUM_FONT_SIZE);
        let root_size = root_font_size.unwrap_or(values::MEDIUM_FONT_SIZE);
        let font_size = properties
            .get("font-size")
            .and_then(|value| values::compute_font_size(value, parent_font_size, root_size))
            .unwrap_or(parent_font_size);
        properties.insert("font-size".to_owned(), values::format_px(font_size));
        if root_font_size.is_none() {
            root_font_size = Some(font_size);
        }
        computed.push((
            NodeID::new(index),
            properties
                .into_iter()
                .map(|(property, value)| Declaration { property, value })
                .collect(),
        ));
    }
    ComputedStyles { styles: computed }
}

/// Splits a trailing `!important` annotation from a declaration value.
fn split_important(value: &str) -> (&str, bool) {
    let trimmed = value.trim_end();
    let lower = trimmed.to_ascii_lowercase();
    if let Some(before) = lower.strip_suffix("important") {
        let before = before.trim_end();
        if let Some(rest) = before.strip_suffix('!') {
            return (trimmed[..rest.len()].trim_end(), true);
        }
    }
    (trimmed, false)
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
            | "visibility"
    )
}

fn attribute_value<'a>(element: &'a html::ElementData, name: &str) -> Option<&'a str> {
    element
        .get_attributes()
        .iter()
        .find(|attribute| attribute.get_name() == name)
        .map(|attribute| attribute.get_value().as_str())
}

fn matches_selector(dom: &HTMLDocument, node_id: NodeID, selector: &str) -> bool {
    let parts = selector.split_whitespace().collect::<Vec<_>>();
    let Some(last) = parts.last() else {
        return false;
    };
    if !matches_compound(dom, node_id, last) {
        return false;
    }
    let mut current = node_id;
    for part in parts.iter().rev().skip(1) {
        let mut ancestor = dom.nodes[current.get()].parent;
        let mut found = None;
        while let Some(candidate) = ancestor {
            if matches_compound(dom, candidate, part) {
                found = Some(candidate);
                break;
            }
            ancestor = dom.nodes[candidate.get()].parent;
        }
        let Some(found) = found else { return false };
        current = found;
    }
    true
}

fn matches_compound(dom: &HTMLDocument, node_id: NodeID, selector: &str) -> bool {
    let Some(node) = dom.node(node_id) else {
        return false;
    };
    let NodeKind::Element(element) = &node.kind else {
        return false;
    };
    let mut index = 0;
    if selector
        .as_bytes()
        .first()
        .is_some_and(|byte| byte.is_ascii_alphabetic())
    {
        let end = selector
            .find(['#', '.', ':', '['])
            .unwrap_or(selector.len());
        if element.get_name() != &selector[..end].to_ascii_lowercase() {
            return false;
        }
        index = end;
    }
    while index < selector.len() {
        let marker = selector.as_bytes()[index] as char;
        if marker == '#' || marker == '.' {
            let start = index + 1;
            let end = selector[start..]
                .find(['#', '.', ':', '['])
                .map_or(selector.len(), |offset| start + offset);
            let value = &selector[start..end];
            let matches = if marker == '#' {
                attribute_value(element, "id") == Some(value)
            } else {
                attribute_value(element, "class")
                    .is_some_and(|classes| classes.split_whitespace().any(|class| class == value))
            };
            if !matches {
                return false;
            }
            index = end;
        } else {
            return false;
        }
    }
    true
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
}
