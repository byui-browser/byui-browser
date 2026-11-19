//! Cascade and inheritance.
//!
//! Selector matching lives in [`crate::matching`]; the rule hash that limits
//! which rules an element is tested against lives in [`crate::rule_index`].

use crate::animation::{ElementContext, KeyframesRegistry, apply_animations};
use crate::computed::{ComputedStyles, PseudoElement, computed_style};
use crate::matching::{ElementTree, SelectorState, attribute};
use crate::parser::parse_declarations;
use crate::rule_index::{CompiledDeclaration, CompiledSheet, RuleIndex, compile_declarations};
use crate::stylesheet::{
    AtRule, Declaration, Dom, Rule, Selector, Specificity, StyleContext, Stylesheet,
};
use crate::user_agent::{user_agent_compiled, user_agent_stylesheet};
use crate::values;
use html::{NodeID, NodeKind};
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

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
/// pseudo-classes match. `style_document` uses empty interaction state;
/// [`style_document_with_state`] accepts pointer and focus state. History
/// pseudo-classes never match. `::before` and `::after` rules are cascaded
/// separately and read through [`ComputedStyles::pseudo_declarations_for`];
/// other pseudo-elements never match. Conditional group
/// rules such as `@media` and nested CSS rules are expanded before matching.
///
/// CSS animations (`@keyframes` with `animation-*`) are sampled at the start
/// of the document timeline; use [`style_document_at`] to style a later
/// moment.
///
/// Cost: each stylesheet is compiled once per call, and each element is only
/// tested against rules filed under its ID, classes, attribute names, and tag
/// (plus rules with no such key), not against every rule.
pub fn style_document(dom: &Dom, stylesheets: &[Stylesheet]) -> ComputedStyles {
    style_document_with_context(dom, stylesheets, StyleContext::default())
}

/// Styles the document using viewport and media-environment information.
pub fn style_document_with_context(
    dom: &Dom,
    stylesheets: &[Stylesheet],
    context: StyleContext,
) -> ComputedStyles {
    style_document_with_stats_context(
        dom,
        stylesheets,
        SelectorState::default(),
        Duration::ZERO,
        context,
    )
    .0
}

/// Styles the document like [`style_document`], with CSS animations sampled
/// `time` after the origin of the document timeline.
///
/// Every animation starts at the timeline origin (plus its
/// `animation-delay`), so callers pass the time elapsed since the page was
/// first shown. Animated values override normal declarations but not
/// `!important` ones, and they are inherited like other computed values.
/// [`ComputedStyles::running_animations`] tells callers whether styling a
/// later moment could produce different styles. See [`crate::animation`]
/// for the supported features.
pub fn style_document_at(dom: &Dom, stylesheets: &[Stylesheet], time: Duration) -> ComputedStyles {
    style_document_with_state_at(dom, stylesheets, SelectorState::default(), time)
}

/// Applies CSS using the supplied pointer and focus state.
///
/// State node IDs must refer to `dom`. The default [`style_document`] entry
/// point matches no dynamic pseudo-classes. This initial implementation
/// supports `:hover`, `:active`, `:focus`, `:focus-within`, and keyboard-driven
/// `:focus-visible`. CSS animations are sampled at the start of the document
/// timeline; use [`style_document_with_state_at`] to style a later moment.
pub fn style_document_with_state(
    dom: &Dom,
    stylesheets: &[Stylesheet],
    state: SelectorState,
) -> ComputedStyles {
    style_document_with_state_at(dom, stylesheets, state, Duration::ZERO)
}

/// Styles the document with both pointer and focus `state` (as in
/// [`style_document_with_state`]) and CSS animations sampled `time` after the
/// origin of the document timeline (as in [`style_document_at`]).
pub fn style_document_with_state_at(
    dom: &Dom,
    stylesheets: &[Stylesheet],
    state: SelectorState,
    time: Duration,
) -> ComputedStyles {
    style_document_with_stats_context(dom, stylesheets, state, time, StyleContext::default()).0
}

fn style_document_with_stats_context(
    dom: &Dom,
    stylesheets: &[Stylesheet],
    state: SelectorState,
    time: Duration,
    context: StyleContext,
) -> (ComputedStyles, CascadeStats) {
    log::debug!(
        "styling document: {} nodes, {} author stylesheets",
        dom.get_nodes().len(),
        stylesheets.len()
    );
    if log::log_enabled!(log::Level::Debug) {
        let ignored_at_rules = stylesheets
            .iter()
            .flat_map(|sheet| &sheet.at_rules)
            .filter(|rule| rule.keyframes.is_empty())
            .count();
        if ignored_at_rules > 0 {
            // Debug, not warn: the shell caches the rendered page but restyles
            // it on window resize and on page clicks (hit tests), so a warning
            // here would repeat on each of those.
            log::debug!(
                "{ignored_at_rules} at-rules in author stylesheets are not applied by the cascade"
            );
        }
    }

    let author_sheets: Vec<CompiledSheet> = stylesheets
        .iter()
        .map(|sheet| expand_stylesheet(sheet, context))
        .map(|sheet| CompiledSheet::compile(&sheet))
        .collect();
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
    let keyframes = KeyframesRegistry::new(stylesheets);
    let time = time.as_secs_f64();
    let mut running_animations = 0;
    let tree = ElementTree::new(dom, state);
    let mut stats = CascadeStats {
        indexed_selectors: index.rules.len(),
        ..CascadeStats::default()
    };

    let mut computed: Vec<(NodeID, Vec<Declaration>)> = Vec::new();
    let mut pseudo: Vec<((NodeID, PseudoElement), Vec<Declaration>)> = Vec::new();
    let mut root_font_size = None;
    let mut candidates = Vec::new();
    let initial_color = initial_color_value();
    for (node_index, node) in dom.get_nodes().iter().enumerate() {
        let NodeKind::Element(element) = node.get_kind() else {
            continue;
        };
        stats.elements += 1;
        let parent = node
            .get_parent()
            .and_then(|parent| computed_style(&computed, parent));

        // Inline declarations must outlive `cascaded`, which borrows them.
        let inline: Vec<CompiledDeclaration> = match attribute(element, "style") {
            Some(style) => {
                let mut inline_errors = Stylesheet::default();
                compile_declarations(&parse_declarations(style, &mut inline_errors, 0))
            }
            None => Vec::new(),
        };

        let mut cascaded = HashMap::<&str, (CascadePriority, &str)>::new();
        let mut before = HashMap::<&str, (CascadePriority, &str)>::new();
        let mut after = HashMap::<&str, (CascadePriority, &str)>::new();
        index.candidates(element, &mut candidates);
        for &candidate in &candidates {
            let indexed = &index.rules[candidate as usize];
            stats.selectors_checked += 1;
            if !tree.matches(&indexed.rule.selector, node_index) {
                continue;
            }
            stats.selectors_matched += 1;
            log::trace!(
                "rule {} of sheet {} matches node {node_index} (pseudo-element {:?})",
                indexed.rule.rule_index,
                indexed.sheet_index,
                indexed.rule.pseudo
            );
            let target = match indexed.rule.pseudo {
                None => &mut cascaded,
                Some(PseudoElement::Before) => &mut before,
                Some(PseudoElement::After) => &mut after,
            };
            for (position, declaration) in indexed.rule.declarations.iter().enumerate() {
                let level = if declaration.important {
                    indexed.important_level
                } else {
                    indexed.normal_level
                };
                stats.declarations_applied += 1;
                apply(
                    target,
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

        // Animations do not override `!important` declarations.
        let important: Vec<&str> = cascaded
            .iter()
            .filter(|(_, (priority, _))| priority.level >= AUTHOR_IMPORTANT)
            .map(|(property, _)| *property)
            .collect();
        let parent_declarations = parent.map(Vec::as_slice);
        let root_size = root_font_size.unwrap_or(values::MEDIUM_FONT_SIZE);
        let (declarations, font_size) = compute(
            parent_declarations,
            cascaded,
            &initial_color,
            root_size,
            context,
            |properties, parent_font_size| {
                running_animations += apply_animations(
                    &keyframes,
                    properties,
                    time,
                    &ElementContext {
                        parent: parent_declarations,
                        parent_font_size,
                        // Before the root's own font size is known, `rem` in the
                        // root's animations uses the parent size, as for `em`.
                        root_font_size: root_font_size.unwrap_or(parent_font_size),
                        important: &important,
                    },
                );
            },
        );
        if root_font_size.is_none() {
            root_font_size = Some(font_size);
        }
        // A pseudo-element inherits from its originating element.
        let root_size = root_font_size.unwrap_or(values::MEDIUM_FONT_SIZE);
        for (which, declared) in [
            (PseudoElement::Before, before),
            (PseudoElement::After, after),
        ] {
            if declared.is_empty() {
                continue;
            }
            // Animations on pseudo-elements are not supported yet.
            let (pseudo_declarations, _) = compute(
                Some(&declarations),
                declared,
                &initial_color,
                root_size,
                context,
                |_, _| {},
            );
            pseudo.push(((NodeID::new(node_index), which), pseudo_declarations));
        }
        computed.push((NodeID::new(node_index), declarations));
    }
    log::debug!(
        "styled document: {} elements, {} indexed selectors, {} selector checks ({} matched), \
         {} longhand declarations considered, {running_animations} running animations",
        stats.elements,
        stats.indexed_selectors,
        stats.selectors_checked,
        stats.selectors_matched,
        stats.declarations_applied
    );
    (
        ComputedStyles::new(computed, pseudo, running_animations),
        stats,
    )
}

fn expand_stylesheet(sheet: &Stylesheet, context: StyleContext) -> Stylesheet {
    let mut expanded = Stylesheet {
        source: sheet.source.clone(),
        rules: Vec::new(),
        at_rules: Vec::new(),
        errors: sheet.errors.clone(),
    };
    for rule in &sheet.rules {
        expand_rule(rule, None, &mut expanded.rules, context);
    }
    for at_rule in &sheet.at_rules {
        if at_rule_applies(at_rule, context) {
            expand_at_rule(at_rule, None, &mut expanded.rules, context);
        }
    }
    expanded
}

fn expand_at_rule(
    at_rule: &AtRule,
    parent: Option<&str>,
    output: &mut Vec<Rule>,
    context: StyleContext,
) {
    for rule in &at_rule.rules {
        expand_rule(rule, parent, output, context);
    }
    for nested in &at_rule.at_rules {
        if at_rule_applies(nested, context) {
            expand_at_rule(nested, parent, output, context);
        }
    }
}

fn expand_rule(rule: &Rule, parent: Option<&str>, output: &mut Vec<Rule>, context: StyleContext) {
    let selector = match parent {
        Some(parent) if rule.selector.0.contains('&') => rule.selector.0.replace('&', parent),
        Some(parent) => format!("{parent} {}", rule.selector.0.trim()),
        None => rule.selector.0.trim().to_owned(),
    };
    output.push(Rule {
        selector: Selector(selector.clone()),
        declarations: rule.declarations.clone(),
        nested_rules: Vec::new(),
        nested_at_rules: Vec::new(),
    });
    for child in &rule.nested_rules {
        expand_rule(child, Some(&selector), output, context);
    }
    for at_rule in &rule.nested_at_rules {
        if at_rule_applies(at_rule, context) {
            expand_at_rule(at_rule, Some(&selector), output, context);
        }
    }
}

fn at_rule_applies(at_rule: &AtRule, context: StyleContext) -> bool {
    match at_rule.name.as_str() {
        "media" => media_applies(&at_rule.prelude, context),
        "supports" | "layer" | "starting-style" => true,
        // These require layout, container, URL, or document state not owned
        // by the CSS crate's current public API.
        "container" | "scope" | "document" | "-moz-document" => false,
        // @keyframes and descriptor rules do not contribute selectors.
        "keyframes" | "-webkit-keyframes" | "-moz-keyframes" | "font-face" | "page"
        | "property" | "counter-style" => false,
        _ => false,
    }
}

fn media_applies(prelude: &str, context: StyleContext) -> bool {
    prelude.split(',').any(|query| {
        let query = query.trim();
        if query.is_empty() {
            return false;
        }
        let lower = query.to_ascii_lowercase();
        let (negated, query) = lower
            .strip_prefix("not ")
            .map_or((false, lower.as_str()), |q| (true, q));
        let mut ok = true;
        for token in media_query_tokens(query) {
            if token == "and" || token == "only" {
                continue;
            }
            if !token.starts_with('(') {
                ok &= match token.as_str() {
                    "all" => true,
                    "screen" => context.screen,
                    "print" => !context.screen,
                    _ => false,
                };
            } else {
                ok &= media_feature_applies(&token, context);
            }
        }
        if negated { !ok } else { ok }
    })
}

fn media_query_tokens(query: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut depth = 0usize;
    let mut start = None;
    for (index, character) in query.char_indices() {
        if character == '(' {
            depth += 1;
            if start.is_none() {
                start = Some(index);
            }
        }
        if character == ')' {
            depth = depth.saturating_sub(1);
        }
        if depth == 0 && character.is_ascii_whitespace() {
            if let Some(begin) = start.take() {
                tokens.push(query[begin..index].to_owned());
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }
    if let Some(begin) = start {
        tokens.push(query[begin..].to_owned());
    }
    tokens
}

fn media_feature_applies(token: &str, context: StyleContext) -> bool {
    let feature = token.trim_matches(|c| c == '(' || c == ')').trim();
    let Some((name, value)) = feature.split_once(':') else {
        return false;
    };
    let name = name.trim();
    let value = value.trim();
    let actual = match name {
        "width" | "min-width" | "max-width" => context.viewport_width,
        "height" | "min-height" | "max-height" => context.viewport_height,
        _ => return false,
    };
    let wanted = if let Some(length) = values::parse_length(value) {
        length.to_px(&values::LengthContext {
            font_size: values::MEDIUM_FONT_SIZE,
            root_font_size: values::MEDIUM_FONT_SIZE,
            viewport_width: context.viewport_width,
            viewport_height: context.viewport_height,
        })
    } else {
        return false;
    };
    match name {
        "min-width" | "min-height" => actual >= wanted,
        "max-width" | "max-height" => actual <= wanted,
        "width" | "height" => (actual - wanted).abs() < 0.001,
        _ => false,
    }
}

/// Resolves one element's or pseudo-element's cascaded values against its
/// parent's computed declarations.
///
/// Applies inheritance, the CSS-wide keywords, the initial `color`, and an
/// absolute `font-size`. `animate` runs after inheritance and before
/// `font-size` is resolved, with the parent's font size, so animated values
/// take part in the result. Returns the declarations sorted by property, and
/// the computed font size in CSS pixels.
fn compute(
    parent: Option<&[Declaration]>,
    cascaded: HashMap<&str, (CascadePriority, &str)>,
    initial_color: &str,
    root_font_size: f32,
    context: StyleContext,
    animate: impl FnOnce(&mut BTreeMap<String, String>, f32),
) -> (Vec<Declaration>, f32) {
    let parent_value = |property: &str| {
        parent.and_then(|declarations| {
            declarations
                .iter()
                .find(|declaration| declaration.property == property)
                .map(|declaration| declaration.value.as_str())
        })
    };
    let mut properties = BTreeMap::<String, String>::new();
    if let Some(parent) = parent {
        for declaration in parent {
            if is_inherited_property(&declaration.property)
                || declaration.property.starts_with("--")
            {
                properties.insert(declaration.property.clone(), declaration.value.clone());
            }
        }
    }
    properties
        .entry("color".to_owned())
        .or_insert_with(|| initial_color.to_owned());
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
                properties.insert(property.to_owned(), initial_color.to_owned());
            }
            None => {
                properties.remove(property);
            }
        }
    }

    resolve_computed_values(&mut properties);

    let parent_font_size = parent_value("font-size")
        .and_then(|value| {
            values::compute_font_size_with_context(
                value,
                values::MEDIUM_FONT_SIZE,
                values::MEDIUM_FONT_SIZE,
                context.viewport_width,
                context.viewport_height,
            )
        })
        .unwrap_or(values::MEDIUM_FONT_SIZE);
    animate(&mut properties, parent_font_size);
    let font_size = properties
        .get("font-size")
        .and_then(|value| {
            values::compute_font_size_with_context(
                value,
                parent_font_size,
                root_font_size,
                context.viewport_width,
                context.viewport_height,
            )
        })
        .unwrap_or_else(|| {
            log::trace!("font-size not computable; using parent size");
            parent_font_size
        });
    properties.insert("font-size".to_owned(), values::format_px(font_size));
    if let Some(percent) = properties
        .get("line-height")
        .and_then(|value| values::parse_percentage(value))
    {
        properties.insert(
            "line-height".to_owned(),
            values::format_px(font_size * percent / 100.0),
        );
    }
    (
        properties
            .into_iter()
            .map(|(property, value)| Declaration { property, value })
            .collect(),
        font_size,
    )
}

fn resolve_computed_values(properties: &mut BTreeMap<String, String>) {
    let variables: BTreeMap<String, String> = properties
        .iter()
        .filter(|(property, _)| property.starts_with("--"))
        .map(|(property, value)| (property.clone(), value.clone()))
        .collect();
    let property_names: Vec<String> = properties.keys().cloned().collect();

    for property in property_names {
        let Some(value) = properties.get(&property).cloned() else {
            continue;
        };
        let Some(mut value) = substitute_variables(&value, &variables, &mut Vec::new()) else {
            properties.remove(&property);
            continue;
        };
        if !property.starts_with("--")
            && let Some(resolved) = values::resolve_length_function(&value)
        {
            value = resolved;
        }
        properties.insert(property, value);
    }
}

fn substitute_variables(
    value: &str,
    variables: &BTreeMap<String, String>,
    stack: &mut Vec<String>,
) -> Option<String> {
    let mut result = String::new();
    let mut index = 0;

    while index < value.len() {
        if !value[index..].to_ascii_lowercase().starts_with("var(") {
            let character = value[index..].chars().next()?;
            result.push(character);
            index += character.len_utf8();
            continue;
        }

        let end = matching_function_end(value, index)?;
        let arguments = split_function_arguments(&value[index + 4..end - 1]);
        let name = arguments.first()?.trim();
        if !name.starts_with("--") {
            return None;
        }

        let replacement = variables
            .get(name)
            .cloned()
            .or_else(|| arguments.get(1).map(|fallback| fallback.to_string()))?;
        if stack.iter().any(|item| item == name) {
            return None;
        }

        stack.push(name.to_owned());
        let resolved = substitute_variables(&replacement, variables, stack)?;
        stack.pop();
        result.push_str(&resolved);
        index = end;
    }

    Some(result)
}

fn matching_function_end(value: &str, start: usize) -> Option<usize> {
    let open = value[start..].find('(')? + start;
    let mut depth: usize = 0;

    for (offset, character) in value[open..].char_indices() {
        match character {
            '(' => depth += 1,
            ')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(open + offset + 1);
                }
            }
            _ => {}
        }
    }

    None
}

fn split_function_arguments(input: &str) -> Vec<&str> {
    let mut arguments = Vec::new();
    let mut depth: usize = 0;
    let mut start = 0;

    for (index, character) in input.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                arguments.push(input[start..index].trim());
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }

    arguments.push(input[start..].trim());
    arguments
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

pub(crate) fn is_inherited_property(property: &str) -> bool {
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
