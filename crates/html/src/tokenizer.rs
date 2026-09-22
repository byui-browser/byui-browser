use common::ids::NodeId;

use crate::{Dom, LegacyNode, Token};

/// Splits raw markup into tokens.
///
/// Handles text, comments, doctypes, start tags, and end tags.
pub fn tokenize(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut cursor = 0;
    while cursor < input.len() {
        let Some(relative_open) = input[cursor..].find('<') else {
            push_text(&mut tokens, &input[cursor..]);
            break;
        };
        let open = cursor + relative_open;
        push_text(&mut tokens, &input[cursor..open]);
        if input[open..].starts_with("<!--") {
            let Some(end) = input[open + 4..].find("-->") else {
                break;
            };
            tokens.push(Token::Comment(input[open + 4..open + 4 + end].to_owned()));
            cursor = open + 4 + end + 3;
            continue;
        }
        let Some(relative_close) = input[open..].find('>') else {
            break;
        };
        let close = open + relative_close;
        let tag = input[open + 1..close].trim();
        if tag
            .get(..8)
            .is_some_and(|name| name.eq_ignore_ascii_case("!doctype"))
        {
            tokens.push(Token::Doctype(tag[8..].trim().to_ascii_lowercase()));
        } else if let Some(name) = tag.strip_prefix('/') {
            if let Some(name) = tag_name(name) {
                tokens.push(Token::EndTag(name));
            }
        } else if let Some(name) = tag_name(tag.trim_end_matches('/').trim()) {
            tokens.push(Token::StartTag(name));
        }
        cursor = close + 1;
    }
    tokens
}

/// Parses markup into the legacy DOM tree.
///
/// Builds a parent-linked arena tree and recovers from unclosed elements.
pub fn parse(input: &str) -> Dom {
    let mut dom = Dom::default();
    let mut stack = Vec::new();
    for token in tokenize(input) {
        match token {
            Token::StartTag(name) => {
                let id = NodeId::new(dom.nodes.len() as u32);
                let parent = stack.last().copied();
                let is_void = is_void_element(&name);
                dom.nodes.push(LegacyNode {
                    id,
                    name,
                    parent,
                    text: None,
                });
                if !is_void {
                    stack.push(id);
                }
            }
            Token::EndTag(name) => {
                if let Some(position) = stack
                    .iter()
                    .rposition(|id| dom.nodes[id.index() as usize].name == name)
                {
                    stack.truncate(position);
                }
            }
            Token::Text(text) if !text.is_empty() => {
                let id = NodeId::new(dom.nodes.len() as u32);
                let parent = stack.last().copied();
                dom.nodes.push(LegacyNode {
                    id,
                    name: "#text".to_owned(),
                    parent,
                    text: Some(text),
                });
            }
            Token::Text(_) | Token::Doctype(_) | Token::Comment(_) => {}
        }
    }
    dom
}

fn push_text(tokens: &mut Vec<Token>, text: &str) {
    if !text.is_empty() {
        tokens.push(Token::Text(text.to_owned()));
    }
}

fn is_void_element(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn tag_name(tag: &str) -> Option<String> {
    tag.split_whitespace()
        .next()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_ascii_lowercase)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_no_tokens() {
        assert!(tokenize("").is_empty());
    }

    #[test]
    fn empty_input_yields_empty_dom() {
        assert_eq!(parse(""), Dom::default());
    }
}
