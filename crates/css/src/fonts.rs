//! Font-related values: `@font-face` rules (CSS Fonts 4 §4) and the
//! `font-family` property (§3.1).

use crate::Stylesheet;

/// One `@font-face` rule: a downloadable font face and the descriptors used
/// to match it against elements' `font-family`, `font-weight`, and
/// `font-style`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFaceRule {
    /// The `font-family` descriptor, unquoted, as written.
    pub family: String,
    /// The `src` descriptor's alternatives in order of preference.
    pub sources: Vec<FontSource>,
    /// The inclusive `font-weight` range the face covers: `normal` is
    /// `(400, 400)`, `bold` `(700, 700)`, `100 900` a variable font's range.
    pub weight: (u16, u16),
    /// Whether `font-style` is `italic` or `oblique`.
    pub italic: bool,
}

/// One alternative in an `@font-face` `src` list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontSource {
    /// `url(…)`, with the lowercased `format(…)` hint when one is given
    /// (such as `woff2` or `truetype`). The URL is unresolved: relative URLs
    /// resolve against the stylesheet's URL.
    Url {
        /// The URL text inside `url(…)`, unquoted.
        url: String,
        /// The lowercased `format(…)` hint, if any.
        format: Option<String>,
    },
    /// `local(…)`: a font installed on the user's system, by full name.
    Local(String),
}

impl FontSource {
    /// Whether this is a URL whose `format()` hint, if any, names a font
    /// format the engine can load: `woff2`, `woff`, `truetype`, `opentype`,
    /// or `collection`.
    pub fn is_loadable_url(&self) -> bool {
        match self {
            Self::Url { format: None, .. } => true,
            Self::Url {
                format: Some(format),
                ..
            } => matches!(
                format.as_str(),
                "woff2" | "woff" | "truetype" | "opentype" | "collection"
            ),
            Self::Local(_) => false,
        }
    }
}

/// The valid top-level `@font-face` rules of `sheet`, in source order.
///
/// A rule needs a `font-family` and at least one `src` entry; others are
/// skipped. Rules nested in `@media` or `@supports` are not returned, and
/// `unicode-range`, `font-display`, and `font-stretch` are ignored.
pub fn font_face_rules(sheet: &Stylesheet) -> Vec<FontFaceRule> {
    sheet
        .at_rules
        .iter()
        .filter(|rule| rule.name == "font-face")
        .filter_map(|rule| {
            let get = |name: &str| {
                rule.declarations
                    .iter()
                    .rev()
                    .find(|declaration| declaration.property == name)
                    .map(|declaration| declaration.value.trim())
            };
            let family = parse_font_family_list(get("font-family")?)
                .into_iter()
                .next()?;
            let sources = parse_sources(get("src")?);
            if sources.is_empty() {
                log::debug!("@font-face for {family:?} has no usable src; skipped");
                return None;
            }
            Some(FontFaceRule {
                family,
                sources,
                weight: get("font-weight").map_or((400, 400), parse_weight_range),
                italic: get("font-style").is_some_and(is_italic),
            })
        })
        .collect()
}

/// Parses a `font-family` value into family names in order of preference.
///
/// Quoted names are unquoted; unquoted names have their identifiers joined
/// with single spaces. Generic families (`serif`, `monospace`, ...) are
/// returned like any other name. `inherit` and similar keywords yield an
/// empty list.
pub fn parse_font_family_list(value: &str) -> Vec<String> {
    let value = value.trim();
    if ["inherit", "initial", "unset", "revert", "revert-layer"]
        .iter()
        .any(|keyword| value.eq_ignore_ascii_case(keyword))
    {
        return Vec::new();
    }
    split_top_level(value, ',')
        .into_iter()
        .filter_map(|entry| {
            let entry = entry.trim();
            let name = match unquote(entry) {
                Some(quoted) => quoted.to_owned(),
                None => entry.split_ascii_whitespace().collect::<Vec<_>>().join(" "),
            };
            (!name.is_empty()).then_some(name)
        })
        .collect()
}

/// Parses a `font-weight` property value to a number from 1 to 1000.
///
/// `bolder` and `lighter` depend on the parent's weight, which is not known
/// here: they map to 700 and 100. Invalid values give `None`.
pub fn parse_font_weight(value: &str) -> Option<u16> {
    let value = value.trim();
    match value.to_ascii_lowercase().as_str() {
        "normal" => Some(400),
        "bold" | "bolder" => Some(700),
        "lighter" => Some(100),
        _ => value
            .parse::<f32>()
            .ok()
            .filter(|weight| (1.0..=1000.0).contains(weight))
            .map(|weight| weight.round() as u16),
    }
}

fn parse_weight_range(value: &str) -> (u16, u16) {
    let mut parts = value.split_ascii_whitespace().filter_map(parse_font_weight);
    match (parts.next(), parts.next()) {
        (Some(low), Some(high)) => (low.min(high), low.max(high)),
        (Some(weight), None) => (weight, weight),
        _ => (400, 400),
    }
}

/// Whether a `font-style` value selects a slanted face.
pub fn is_italic(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    value == "italic" || value.starts_with("oblique")
}

fn parse_sources(value: &str) -> Vec<FontSource> {
    split_top_level(value, ',')
        .into_iter()
        .filter_map(|entry| {
            let entry = entry.trim();
            if let Some(name) = function_argument(entry, "local") {
                return Some(FontSource::Local(unquote(name).unwrap_or(name).to_owned()));
            }
            let url = function_argument(entry, "url")?;
            let url = unquote(url).unwrap_or(url).to_owned();
            let after = &entry[entry.find(')')? + 1..];
            let format = function_argument(after.trim(), "format")
                .map(|format| unquote(format).unwrap_or(format).to_ascii_lowercase());
            Some(FontSource::Url { url, format })
        })
        .collect()
}

/// The text inside `name(…)` when `text` starts with that function
/// (ignoring ASCII case), trimmed.
fn function_argument<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let open = text.find('(')?;
    if !text[..open].trim().eq_ignore_ascii_case(name) {
        return None;
    }
    let mut quote = None;
    for (index, character) in text[open + 1..].char_indices() {
        match (quote, character) {
            (Some(open), _) if character == open => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(character),
            (None, ')') => return Some(text[open + 1..open + 1 + index].trim()),
            _ => {}
        }
    }
    None
}

/// `text` without its surrounding matching quotes, or `None` when unquoted.
fn unquote(text: &str) -> Option<&str> {
    let quote = text.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    text.strip_prefix(quote)?.strip_suffix(quote)
}

/// Splits `text` on `separator` outside quotes and parentheses.
fn split_top_level(text: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut quote = None;
    let mut depth = 0_u32;
    let mut start = 0;
    for (index, character) in text.char_indices() {
        match (quote, character) {
            (Some(open), _) if character == open => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(character),
            (None, '(') => depth += 1,
            (None, ')') => depth = depth.saturating_sub(1),
            (None, _) if character == separator && depth == 0 => {
                parts.push(&text[start..index]);
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_stylesheet;

    #[test]
    fn font_face_rules_parse_family_sources_and_descriptors() {
        let sheet = parse_stylesheet(
            r#"@font-face {
                 font-family: "Open Sans";
                 src: local("Open Sans"), url('fonts/open.woff2') format("woff2"),
                      url(fonts/open.ttf) format('TrueType'), url(x.eot) format("embedded-opentype");
                 font-weight: 300 800;
                 font-style: italic;
               }
               @font-face { font-family: NoSource }
               @font-face { src: url(no-family.woff) }
               @font-face { font-family: Plain Name; src: url(p.woff) }"#,
        );
        let rules = font_face_rules(&sheet);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].family, "Open Sans");
        assert_eq!(rules[0].weight, (300, 800));
        assert!(rules[0].italic);
        assert_eq!(
            rules[0].sources,
            [
                FontSource::Local("Open Sans".into()),
                FontSource::Url {
                    url: "fonts/open.woff2".into(),
                    format: Some("woff2".into())
                },
                FontSource::Url {
                    url: "fonts/open.ttf".into(),
                    format: Some("truetype".into())
                },
                FontSource::Url {
                    url: "x.eot".into(),
                    format: Some("embedded-opentype".into())
                },
            ]
        );
        let loadable: Vec<_> = rules[0]
            .sources
            .iter()
            .map(FontSource::is_loadable_url)
            .collect();
        assert_eq!(loadable, [false, true, true, false]);
        assert_eq!(rules[1].family, "Plain Name");
        assert_eq!((rules[1].weight, rules[1].italic), ((400, 400), false));
    }

    #[test]
    fn font_family_lists_unquote_and_normalize_names() {
        assert_eq!(
            parse_font_family_list(r#" "My Font", Times   New Roman ,'a,b', serif"#),
            ["My Font", "Times New Roman", "a,b", "serif"]
        );
        assert!(parse_font_family_list("inherit").is_empty());
    }

    #[test]
    fn font_weights_and_styles() {
        assert_eq!(parse_font_weight("bold"), Some(700));
        assert_eq!(parse_font_weight("650"), Some(650));
        assert_eq!(parse_font_weight("0"), None);
        assert_eq!(parse_font_weight("heavy"), None);
        assert_eq!(parse_weight_range("bold"), (700, 700));
        assert_eq!(parse_weight_range("900 100"), (100, 900));
        assert!(is_italic("oblique 10deg"));
        assert!(!is_italic("normal"));
    }
}
