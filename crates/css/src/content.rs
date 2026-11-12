//! The `content` property of `::before` and `::after` (CSS Content 3 §2).

use crate::tokenizer::{TokenKind, tokenize};

/// Whether a computed `content` value makes a `::before` or `::after`
/// pseudo-element generate a box. `none` and `normal` (which computes to
/// `none` on these pseudo-elements) do not.
pub(crate) fn generates_box(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && !value.eq_ignore_ascii_case("none")
        && !value.eq_ignore_ascii_case("normal")
}

/// Returns the text a `content` value generates for a `::before` or `::after`
/// pseudo-element, or `None` when the value generates no box (`none`,
/// `normal`, or an empty or malformed value).
///
/// `attribute` looks up an attribute of the originating element by its
/// ASCII-lowercased name, for `attr()`.
///
/// Supported components, concatenated in order: quoted strings (with CSS
/// escapes such as `"\201C"`), `attr(name)` with an optional string fallback
/// (`attr(name, "fallback")`), and the quote keywords. `open-quote` and
/// `close-quote` always produce `“` and `”`: the `quotes` property and quote
/// nesting depth are not supported. `no-open-quote` and `no-close-quote`
/// produce nothing.
///
/// Limitations: images (`url()`, gradients), `counter()`/`counters()`, and
/// other functions contribute no text, although the pseudo-element still
/// generates its box. Alternative text after `/` is ignored.
pub fn generated_content_text(
    value: &str,
    attribute: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    if !generates_box(value) {
        return None;
    }
    let tokens = tokenize(value);
    let mut tokens = tokens.iter().map(|token| &token.kind);
    let mut text = String::new();
    while let Some(token) = tokens.next() {
        match token {
            TokenKind::String(string) => text.push_str(string),
            TokenKind::BadString => {
                log::debug!("ignoring malformed content value {value:?}");
                return None;
            }
            TokenKind::Ident(keyword) => {
                if keyword.eq_ignore_ascii_case("open-quote") {
                    text.push('\u{201C}');
                } else if keyword.eq_ignore_ascii_case("close-quote") {
                    text.push('\u{201D}');
                } else if !keyword.eq_ignore_ascii_case("no-open-quote")
                    && !keyword.eq_ignore_ascii_case("no-close-quote")
                {
                    log::trace!("unsupported content keyword `{keyword}` generates no text");
                }
            }
            TokenKind::Function(name) => {
                // The function's arguments, up to its closing parenthesis.
                let mut depth = 1_usize;
                let mut arguments = Vec::new();
                for inner in tokens.by_ref() {
                    match inner {
                        TokenKind::Function(_) | TokenKind::OpenParen => depth += 1,
                        TokenKind::CloseParen => depth -= 1,
                        _ => {}
                    }
                    if depth == 0 {
                        break;
                    }
                    arguments.push(inner);
                }
                if name.eq_ignore_ascii_case("attr") {
                    text.push_str(&attr_text(&arguments, &attribute));
                } else {
                    log::trace!("unsupported content function `{name}()` generates no text");
                }
            }
            // Alternative text for speech follows the slash.
            TokenKind::Delim('/') => break,
            _ => {}
        }
    }
    Some(text)
}

/// The text of `attr(name)` or `attr(name, "fallback")`: the attribute's
/// value, else the fallback string, else nothing.
fn attr_text(arguments: &[&TokenKind<'_>], attribute: &impl Fn(&str) -> Option<String>) -> String {
    let name = arguments.iter().find_map(|token| match token {
        TokenKind::Ident(name) => Some(name.to_ascii_lowercase()),
        _ => None,
    });
    let fallback = arguments
        .iter()
        .skip_while(|token| !matches!(token, TokenKind::Comma))
        .find_map(|token| match token {
            TokenKind::String(fallback) => Some(fallback.to_string()),
            _ => None,
        });
    name.and_then(|name| attribute(&name))
        .or(fallback)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> Option<String> {
        generated_content_text(value, |name| (name == "title").then(|| "Tip".to_owned()))
    }

    #[test]
    fn none_and_normal_generate_no_box() {
        assert_eq!(text("none"), None);
        assert_eq!(text(" NORMAL "), None);
        assert_eq!(text(""), None);
        assert_eq!(text("\"unterminated\n"), None);
    }

    #[test]
    fn strings_attributes_and_quotes_are_concatenated() {
        assert_eq!(text("\"\"").as_deref(), Some(""));
        assert_eq!(text("'a' \"b\"").as_deref(), Some("ab"));
        assert_eq!(text("\"\\201C\"").as_deref(), Some("\u{201C}"));
        assert_eq!(text("\"(\" attr(TITLE) \")\"").as_deref(), Some("(Tip)"));
        assert_eq!(text("attr(missing)").as_deref(), Some(""));
        assert_eq!(text("attr(missing, 'none')").as_deref(), Some("none"));
        assert_eq!(
            text("open-quote 'x' close-quote no-close-quote").as_deref(),
            Some("\u{201C}x\u{201D}")
        );
    }

    #[test]
    fn unsupported_components_generate_a_box_without_text() {
        assert_eq!(text("counter(item) '. '").as_deref(), Some(". "));
        assert_eq!(text("url(marker.png)").as_deref(), Some(""));
        assert_eq!(text("'>' / 'alternative'").as_deref(), Some(">"));
    }
}
