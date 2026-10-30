//! Forgiving CSS stylesheet parser.
//!
//! Valid rules are kept when a neighboring rule is malformed. Malformed pieces
//! are reported as [`ParseError`] values.

use crate::stylesheet::{AtRule, Declaration, ParseError, Rule, Selector, Stylesheet};

/// Parse CSS and retain all valid rules while collecting recoverable errors.
pub fn parse_stylesheet(input: &str) -> Stylesheet {
    let (stylesheet, _) = parse_stylesheet_with_errors(input);
    stylesheet
}

/// Parse CSS and return both the useful AST and diagnostics.
pub fn parse_stylesheet_with_errors(input: &str) -> (Stylesheet, Vec<ParseError>) {
    let source = remove_comments(input);
    let mut stylesheet = Stylesheet::default();
    parse_rule_list(&source, &mut stylesheet, None);
    let errors = stylesheet.errors.clone();
    (stylesheet, errors)
}

fn parse_rule_list(source: &str, stylesheet: &mut Stylesheet, base_offset: Option<usize>) {
    let mut cursor = 0;
    while cursor < source.len() {
        cursor += source[cursor..]
            .chars()
            .take_while(|character| character.is_whitespace())
            .map(char::len_utf8)
            .sum::<usize>();
        if cursor >= source.len() {
            break;
        }

        let Some((boundary, delimiter)) = find_boundary(&source[cursor..]) else {
            stylesheet.errors.push(ParseError {
                message: "unfinished CSS rule; expected `;` or `{`".into(),
                offset: base_offset.unwrap_or(0) + cursor,
            });
            break;
        };
        let boundary = cursor + boundary;
        let prelude = source[cursor..boundary].trim();
        if prelude.is_empty() {
            cursor = boundary + delimiter.len_utf8();
            continue;
        }

        match delimiter {
            ';' => {
                if prelude.starts_with('@') {
                    stylesheet.at_rules.push(parse_at_rule(prelude, None));
                } else {
                    stylesheet.errors.push(ParseError {
                        message: "a style rule needs a declaration block".into(),
                        offset: base_offset.unwrap_or(0) + cursor,
                    });
                }
                cursor = boundary + 1;
            }
            '{' => {
                let block_start = boundary + 1;
                let Some(block_end) = matching_brace(source, boundary) else {
                    stylesheet.errors.push(ParseError {
                        message: "unclosed `{` in CSS rule".into(),
                        offset: base_offset.unwrap_or(0) + boundary,
                    });
                    break;
                };
                let block = &source[block_start..block_end];
                if prelude.starts_with('@') {
                    stylesheet
                        .at_rules
                        .push(parse_at_rule(prelude, Some(block)));
                } else {
                    let declarations = parse_declarations(
                        block,
                        stylesheet,
                        base_offset.unwrap_or(0) + block_start,
                    );
                    for selector in split_selectors(prelude) {
                        stylesheet.rules.push(Rule {
                            selector: Selector(selector),
                            declarations: declarations.clone(),
                        });
                    }
                }
                cursor = block_end + 1;
            }
            '}' => {
                stylesheet.errors.push(ParseError {
                    message: "unexpected `}`".into(),
                    offset: base_offset.unwrap_or(0) + boundary,
                });
                cursor = boundary + 1;
            }
            _ => unreachable!(),
        }
    }
}

fn parse_at_rule(prelude: &str, block: Option<&str>) -> AtRule {
    let mut pieces = prelude[1..].splitn(2, char::is_whitespace);
    let name = pieces.next().unwrap_or_default().to_ascii_lowercase();
    let prelude = pieces.next().unwrap_or_default().trim().to_owned();
    let mut rule = AtRule {
        name,
        prelude,
        rules: Vec::new(),
        declarations: Vec::new(),
        block: block.map(str::to_owned),
    };
    if let Some(block) = block {
        if matches!(
            rule.name.as_str(),
            "media" | "supports" | "layer" | "document" | "container"
        ) {
            let mut nested = Stylesheet::default();
            parse_rule_list(block, &mut nested, None);
            rule.rules = nested.rules;
        } else {
            let mut nested = Stylesheet::default();
            rule.declarations = parse_declarations(block, &mut nested, 0);
        }
    }
    rule
}

fn find_boundary(source: &str) -> Option<(usize, char)> {
    let mut quote = None;
    let mut escaped = false;
    let mut parentheses: usize = 0;
    let mut brackets: usize = 0;
    for (index, character) in source.char_indices() {
        if let Some(open_quote) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == open_quote {
                quote = None;
            }
            continue;
        }
        match character {
            '"' | '\'' => quote = Some(character),
            '(' => parentheses += 1,
            ')' => parentheses = parentheses.saturating_sub(1),
            '[' => brackets += 1,
            ']' => brackets = brackets.saturating_sub(1),
            ';' | '{' | '}' if parentheses == 0 && brackets == 0 => {
                return Some((index, character));
            }
            _ => {}
        }
    }
    None
}

fn matching_brace(source: &str, opening: usize) -> Option<usize> {
    let mut depth = 1;
    let mut quote = None;
    let mut escaped = false;
    for (relative, character) in source[opening + 1..].char_indices() {
        let index = opening + 1 + relative;
        if let Some(open_quote) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == open_quote {
                quote = None;
            }
            continue;
        }
        match character {
            '"' | '\'' => quote = Some(character),
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_selectors(input: &str) -> Vec<String> {
    split_top_level(input, ',')
        .into_iter()
        .map(|selector| selector.trim().to_owned())
        .filter(|selector| !selector.is_empty())
        .collect()
}

pub(crate) fn parse_declarations(
    input: &str,
    stylesheet: &mut Stylesheet,
    offset: usize,
) -> Vec<Declaration> {
    let mut declarations = Vec::new();
    for statement in split_top_level(input, ';') {
        let statement = statement.trim();
        if statement.is_empty() {
            continue;
        }
        let Some(colon) = top_level_position(statement, ':') else {
            stylesheet.errors.push(ParseError {
                message: "declaration is missing `:`".into(),
                offset,
            });
            continue;
        };
        let property = statement[..colon].trim();
        let value = statement[colon + 1..].trim();
        if property.is_empty() || value.is_empty() {
            stylesheet.errors.push(ParseError {
                message: "declaration has an empty property or value".into(),
                offset,
            });
            continue;
        }
        declarations.push(Declaration {
            property: property.to_ascii_lowercase(),
            value: value.to_owned(),
        });
    }
    declarations
}

fn split_top_level(input: &str, delimiter: char) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut quote = None;
    let mut escaped = false;
    let mut parentheses: usize = 0;
    let mut brackets: usize = 0;
    for (index, character) in input.char_indices() {
        if let Some(open_quote) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == open_quote {
                quote = None;
            }
            continue;
        }
        match character {
            '"' | '\'' => quote = Some(character),
            '(' => parentheses += 1,
            ')' => parentheses = parentheses.saturating_sub(1),
            '[' => brackets += 1,
            ']' => brackets = brackets.saturating_sub(1),
            character if character == delimiter && parentheses == 0 && brackets == 0 => {
                pieces.push(&input[start..index]);
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    pieces.push(&input[start..]);
    pieces
}

fn top_level_position(input: &str, wanted: char) -> Option<usize> {
    let pieces = split_top_level(input, wanted);
    if pieces.len() > 1 {
        Some(pieces[0].len())
    } else {
        None
    }
}

/// Remove comments without damaging `/* ... */` text inside a quoted string.
fn remove_comments(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut index = 0;
    let mut quote = None;
    let mut escaped = false;
    while index < bytes.len() {
        let character = bytes[index] as char;
        if let Some(open_quote) = quote {
            result.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == open_quote {
                quote = None;
            }
            index += 1;
        } else if character == '"' || character == '\'' {
            quote = Some(character);
            result.push(character);
            index += 1;
        } else if index + 1 < bytes.len() && &bytes[index..index + 2] == b"/*" {
            index += 2;
            while index + 1 < bytes.len() && &bytes[index..index + 2] != b"*/" {
                index += 1;
            }
            index = (index + 2).min(bytes.len());
        } else {
            result.push(character);
            index += 1;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comments_strings_functions_and_important_values() {
        let sheet = parse_stylesheet(
            r#"/* comment */ body, .hero { background: url("a;b.png"); content: "}"; color: rgb(1, 2, 3) !important; }"#,
        );
        assert_eq!(sheet.rules.len(), 2);
        assert_eq!(sheet.rules[0].declarations.len(), 3);
        assert!(sheet.errors.is_empty());
    }

    #[test]
    fn parses_at_rules_and_nested_rules() {
        let sheet = parse_stylesheet(
            "@media screen and (min-width: 10px) { body { color: red; } } @import url('x.css');",
        );
        assert_eq!(sheet.at_rules.len(), 2);
        assert_eq!(sheet.at_rules[0].name, "media");
        assert_eq!(sheet.at_rules[0].rules.len(), 1);
        assert_eq!(sheet.at_rules[1].prelude, "url('x.css')");
    }

    #[test]
    fn recovers_from_bad_declaration() {
        let (sheet, errors) =
            parse_stylesheet_with_errors("p { color: red; broken; background: blue; }");
        assert_eq!(sheet.rules[0].declarations.len(), 2);
        assert_eq!(errors.len(), 1);
    }
}
