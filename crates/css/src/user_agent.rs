//! Bundled user-agent stylesheet.

use crate::color::{CssColor, parse_color};
use crate::parser::parse_stylesheet;
use crate::stylesheet::Stylesheet;
use std::sync::OnceLock;

/// Returns the bundled user-agent stylesheet parsed from `user_agent.css`.
///
/// The file is embedded at build time and parsed once.
pub fn user_agent_stylesheet() -> &'static Stylesheet {
    static SHEET: OnceLock<Stylesheet> = OnceLock::new();
    SHEET.get_or_init(|| {
        let sheet = parse_stylesheet(include_str!("user_agent.css"));
        assert!(sheet.errors.is_empty(), "bundled user-agent CSS must parse");
        sheet
    })
}

/// Returns a supported color default from the bundled `html` user-agent rule.
///
/// Callers use `color` for the initial text color. The rule deliberately has no
/// `background-color`: per CSS Backgrounds 3 §2.11.2 the canvas takes the root
/// or `body` background, and the user agent's default canvas color is applied
/// by `paint`. Missing or unsupported properties return `None`.
pub fn user_agent_default_color(property: &str) -> Option<CssColor> {
    user_agent_stylesheet()
        .rules
        .iter()
        .find(|rule| rule.selector.0 == "html")?
        .declarations
        .iter()
        .find(|declaration| declaration.property == property)
        .and_then(|declaration| parse_color(&declaration.value))
}
