//! Precompiled style rules and the rule hash used to find match candidates.
//!
//! Testing every element against every rule costs `elements × rules` selector
//! matches, which is tens of millions for large sites. Like other engines, the
//! cascade instead files each rule under one key of its rightmost compound:
//! an ID, a class, an attribute name, or a tag name, falling back to a
//! "universal" list. An element only needs to be tested against the rules
//! filed under its own ID, classes, attribute names, and tag, plus the
//! universal ones, because a selector can only match an element its rightmost
//! compound matches.
//!
//! Compiling also does once-per-rule work up front: selectors are parsed,
//! specificity is computed, `!important` is split off, and shorthands are
//! expanded to longhands.

use crate::matching::compound_never_matches;
use crate::selector::{ComplexSelector, CompoundSelector, SimpleSelector, parse_selector_list};
use crate::shorthands;
use crate::specificity::complex_specificity;
use crate::stylesheet::{Declaration, Specificity, Stylesheet};
use html::ElementData;
use std::collections::HashMap;
use std::sync::Arc;

/// A longhand declaration ready for the cascade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompiledDeclaration {
    /// Longhand property name.
    pub(crate) property: String,
    /// Specified value text without `!important`.
    pub(crate) value: String,
    /// Whether the declaration was marked `!important`.
    pub(crate) important: bool,
}

/// Splits `!important` from a declaration and expands shorthands.
pub(crate) fn compile_declarations(declarations: &[Declaration]) -> Vec<CompiledDeclaration> {
    let mut compiled = Vec::with_capacity(declarations.len());
    for declaration in declarations {
        let (value, important) = split_important(&declaration.value);
        // Only build a new declaration when `!important` must be removed.
        let expanded = if important {
            shorthands::expand(&Declaration {
                property: declaration.property.clone(),
                value: value.to_owned(),
            })
        } else {
            shorthands::expand(declaration)
        };
        compiled.extend(expanded.into_iter().map(|longhand| CompiledDeclaration {
            property: longhand.property,
            value: longhand.value,
            important,
        }));
    }
    compiled
}

/// Splits a trailing `!important` annotation from a declaration value.
fn split_important(value: &str) -> (&str, bool) {
    let trimmed = value.trim_end();
    let split = trimmed.len().saturating_sub("important".len());
    if trimmed.len() >= "important".len() && trimmed.is_char_boundary(split) {
        let (before, word) = trimmed.split_at(split);
        if word.eq_ignore_ascii_case("important")
            && let Some(rest) = before.trim_end().strip_suffix('!')
        {
            return (rest.trim_end(), true);
        }
    }
    (trimmed, false)
}

/// One complex selector of a stylesheet rule, with its declarations.
#[derive(Debug)]
pub(crate) struct CompiledRule {
    /// The parsed selector.
    pub(crate) selector: ComplexSelector,
    /// Specificity of `selector`.
    pub(crate) specificity: Specificity,
    /// Index of the source rule in [`Stylesheet::rules`] (source order).
    pub(crate) rule_index: usize,
    /// Expanded declarations, shared by rules split from one selector list.
    pub(crate) declarations: Arc<[CompiledDeclaration]>,
}

/// A stylesheet whose rules are ready for indexing.
#[derive(Debug, Default)]
pub(crate) struct CompiledSheet {
    pub(crate) rules: Vec<CompiledRule>,
}

impl CompiledSheet {
    /// Compiles the style rules of `sheet`. Rules whose selector is invalid,
    /// or can never match in this engine, are left out.
    pub(crate) fn compile(sheet: &Stylesheet) -> Self {
        let mut rules = Vec::with_capacity(sheet.rules.len());
        let mut previous: Option<(&[Declaration], Arc<[CompiledDeclaration]>)> = None;
        let mut skipped = 0_usize;
        for (rule_index, rule) in sheet.rules.iter().enumerate() {
            let list = match parse_selector_list(&rule.selector.0) {
                Ok(list) => list,
                Err(error) => {
                    log::debug!(
                        "skipping rule with invalid selector `{}`: {error}",
                        rule.selector.0
                    );
                    continue;
                }
            };
            let before = skipped;
            let selectors: Vec<ComplexSelector> = list
                .selectors
                .into_iter()
                .filter(|selector| {
                    let never = selector.compounds.iter().any(compound_never_matches);
                    skipped += usize::from(never);
                    !never
                })
                .collect();
            if selectors.is_empty() {
                log::trace!(
                    "rule `{}` can never match ({} selectors skipped)",
                    rule.selector.0,
                    skipped - before
                );
                continue;
            }
            // Rules split from one selector list carry equal declarations;
            // compile them once.
            let declarations = match &previous {
                Some((source, compiled)) if *source == rule.declarations.as_slice() => {
                    Arc::clone(compiled)
                }
                _ => {
                    let compiled: Arc<[CompiledDeclaration]> =
                        compile_declarations(&rule.declarations).into();
                    previous = Some((rule.declarations.as_slice(), Arc::clone(&compiled)));
                    compiled
                }
            };
            for selector in selectors {
                rules.push(CompiledRule {
                    specificity: complex_specificity(&selector),
                    selector,
                    rule_index,
                    declarations: Arc::clone(&declarations),
                });
            }
        }
        log::debug!(
            "compiled {} selectors; skipped {skipped} that can never match",
            rules.len()
        );
        Self { rules }
    }
}

/// A rule filed in the index, with its origin's cascade levels.
pub(crate) struct IndexedRule<'r> {
    pub(crate) rule: &'r CompiledRule,
    /// Position of the rule's stylesheet among all applied sheets.
    pub(crate) sheet_index: usize,
    /// Cascade level of the rule's normal declarations.
    pub(crate) normal_level: u8,
    /// Cascade level of the rule's `!important` declarations.
    pub(crate) important_level: u8,
}

/// The rule hash: compiled rules filed by a key of their rightmost compound.
#[derive(Default)]
pub(crate) struct RuleIndex<'r> {
    pub(crate) rules: Vec<IndexedRule<'r>>,
    by_id: HashMap<&'r str, Vec<u32>>,
    by_class: HashMap<&'r str, Vec<u32>>,
    by_attribute: HashMap<String, Vec<u32>>,
    by_tag: HashMap<String, Vec<u32>>,
    universal: Vec<u32>,
}

enum Key<'r> {
    Id(&'r str),
    Class(&'r str),
    Attribute(String),
    Tag(String),
    Universal,
}

fn key_for(compound: &CompoundSelector) -> Key<'_> {
    let simples = &compound.simples;
    if let Some(id) = simples.iter().find_map(|simple| match simple {
        SimpleSelector::Id(id) => Some(id.as_str()),
        _ => None,
    }) {
        return Key::Id(id);
    }
    if let Some(class) = simples.iter().find_map(|simple| match simple {
        SimpleSelector::Class(class) => Some(class.as_str()),
        _ => None,
    }) {
        return Key::Class(class);
    }
    if let Some(name) = simples.iter().find_map(|simple| match simple {
        SimpleSelector::Attribute(attribute) => Some(attribute.name.to_ascii_lowercase()),
        _ => None,
    }) {
        return Key::Attribute(name);
    }
    match &compound.type_selector {
        Some(type_selector) if type_selector.name != "*" => {
            Key::Tag(type_selector.name.to_ascii_lowercase())
        }
        _ => Key::Universal,
    }
}

impl<'r> RuleIndex<'r> {
    /// Files every rule of `sheet` under the given origin levels.
    pub(crate) fn add_sheet(
        &mut self,
        sheet: &'r CompiledSheet,
        sheet_index: usize,
        normal_level: u8,
        important_level: u8,
    ) {
        for rule in &sheet.rules {
            let position = u32::try_from(self.rules.len()).expect("fewer than 2^32 rules");
            let rightmost = rule
                .selector
                .compounds
                .last()
                .expect("parsed selectors have a compound");
            match key_for(rightmost) {
                Key::Id(id) => self.by_id.entry(id).or_default().push(position),
                Key::Class(class) => self.by_class.entry(class).or_default().push(position),
                Key::Attribute(name) => self.by_attribute.entry(name).or_default().push(position),
                Key::Tag(tag) => self.by_tag.entry(tag).or_default().push(position),
                Key::Universal => self.universal.push(position),
            }
            self.rules.push(IndexedRule {
                rule,
                sheet_index,
                normal_level,
                important_level,
            });
        }
    }

    /// Replaces `candidates` with the rules that might match `element`.
    ///
    /// Every rule whose selector matches `element` is included. Most of the
    /// included rules still need a full selector match.
    pub(crate) fn candidates(&self, element: &ElementData, candidates: &mut Vec<u32>) {
        candidates.clear();
        candidates.extend_from_slice(&self.universal);
        let mut seen_classes: Vec<&str> = Vec::new();
        for attribute in element.get_attributes() {
            let name = attribute.get_name().as_str();
            let value = attribute.get_value().as_str();
            match name {
                "id" => {
                    if let Some(rules) = self.by_id.get(value) {
                        candidates.extend_from_slice(rules);
                    }
                }
                "class" => {
                    for class in value.split_ascii_whitespace() {
                        if seen_classes.contains(&class) {
                            continue;
                        }
                        seen_classes.push(class);
                        if let Some(rules) = self.by_class.get(class) {
                            candidates.extend_from_slice(rules);
                        }
                    }
                }
                _ => {}
            }
            if let Some(rules) = self.by_attribute.get(name) {
                candidates.extend_from_slice(rules);
            }
        }
        let tag = element.get_name();
        let rules = if tag.bytes().any(|byte| byte.is_ascii_uppercase()) {
            self.by_tag.get(&tag.to_ascii_lowercase())
        } else {
            self.by_tag.get(tag.as_str())
        };
        if let Some(rules) = rules {
            candidates.extend_from_slice(rules);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_stylesheet;

    #[test]
    fn splits_important_case_insensitively() {
        assert_eq!(split_important("red !important"), ("red", true));
        assert_eq!(split_important("red ! IMPORTANT "), ("red", true));
        assert_eq!(split_important("red"), ("red", false));
        assert_eq!(split_important("important"), ("important", false));
        assert_eq!(split_important("\"日本語です\""), ("\"日本語です\"", false));
    }

    #[test]
    fn compiles_shared_declarations_once_and_skips_unmatchable_rules() {
        let sheet = parse_stylesheet("a, b, a:hover, p::before { margin: 0 !important }");
        let compiled = CompiledSheet::compile(&sheet);
        assert_eq!(compiled.rules.len(), 3);
        assert!(Arc::ptr_eq(
            &compiled.rules[0].declarations,
            &compiled.rules[1].declarations
        ));
        assert!(Arc::ptr_eq(
            &compiled.rules[0].declarations,
            &compiled.rules[2].declarations
        ));
        assert_eq!(compiled.rules[0].declarations.len(), 4);
        assert!(compiled.rules[0].declarations[0].important);
    }

    #[test]
    fn candidates_come_from_the_elements_own_keys() {
        let sheet = parse_stylesheet(
            "#main {a:b} .card {a:b} [data-x] {a:b} p {a:b} * {a:b} .other {a:b} div {a:b}",
        );
        let compiled = CompiledSheet::compile(&sheet);
        let mut index = RuleIndex::default();
        index.add_sheet(&compiled, 0, 2, 4);
        let dom = html::parse_raw_html("<p id='main' class='card card' data-x='1'>x</p>".into());
        let element = dom
            .nodes
            .iter()
            .find_map(|node| match node.get_kind() {
                html::NodeKind::Element(element) if element.get_name() == "p" => Some(element),
                _ => None,
            })
            .unwrap();
        let mut candidates = Vec::new();
        index.candidates(element, &mut candidates);
        let mut rules: Vec<usize> = candidates
            .iter()
            .map(|candidate| index.rules[*candidate as usize].rule.rule_index)
            .collect();
        rules.sort_unstable();
        assert_eq!(rules, [0, 1, 2, 3, 4]);
    }
}
