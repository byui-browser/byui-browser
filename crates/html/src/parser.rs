use crate::{HTMLDocument, HTMLElement, Location};
use common::ids::NodeId;
use std::collections::BTreeMap;

/// Parses raw HTML into a document containing its element tree.
pub fn parse_raw_html(raw_string: String) -> HTMLDocument {
    let doc_type = raw_string
        .split_once('<')
        .and_then(|(_, rest)| rest.strip_prefix("!DOCTYPE"))
        .and_then(|rest| rest.split_once('>'))
        .map(|(value, _)| value.trim().to_ascii_lowercase())
        .unwrap_or_default();

    let mut elements: Vec<HTMLElement> = Vec::new();
    let mut stack = Vec::<NodeId>::new();
    let mut cursor = 0;

    while let Some(open_offset) = raw_string[cursor..].find('<') {
        let open = cursor + open_offset;
        let Some(close_offset) = raw_string[open..].find('>') else {
            break;
        };
        let close = open + close_offset;
        let tag = raw_string[open + 1..close].trim();
        cursor = close + 1;

        if tag.starts_with('!') || tag.starts_with('?') {
            continue;
        }
        if let Some(name) = tag.strip_prefix('/') {
            let name = name.split_whitespace().next().unwrap_or_default();
            if let Some(position) = stack
                .iter()
                .rposition(|id| elements[id.index() as usize].name == name)
            {
                stack.truncate(position);
            }
            continue;
        }

        let self_closing = tag.ends_with('/');
        let tag = tag.trim_end_matches('/').trim();
        let (name, attributes) = parse_start_tag(tag);
        if name.is_empty() {
            continue;
        }

        let id = NodeId::new(elements.len() as u32);
        let parent = stack.last().copied().unwrap_or(NodeId::new(u32::MAX));
        elements.push(HTMLElement {
            name: name.clone(),
            attributes,
            parent,
            children: Vec::new(),
        });
        if let Some(parent) = stack.last() {
            elements[parent.index() as usize].children.push(id);
        }
        if !self_closing && !is_void_element(&name) {
            stack.push(id);
        }
    }

    HTMLDocument {
        doc_type,
        nodes: (0..elements.len() as u32).map(NodeId::new).collect(),
        location: Location { url: String::new() },
        elements,
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

fn parse_start_tag(tag: &str) -> (String, BTreeMap<String, String>) {
    let mut parts = tag.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or_default().to_ascii_lowercase();
    let mut attributes = BTreeMap::new();
    let mut rest = parts.next().unwrap_or_default().trim();

    while !rest.is_empty() {
        rest = rest.trim_start();
        let Some(name_end) =
            rest.find(|character: char| character.is_whitespace() || character == '=')
        else {
            attributes.insert(rest.to_ascii_lowercase(), String::new());
            break;
        };
        let attribute_name = rest[..name_end].to_ascii_lowercase();
        rest = rest[name_end..].trim_start();
        let mut value = String::new();
        if let Some(after_equals) = rest.strip_prefix('=') {
            rest = after_equals.trim_start();
            if let Some(quote) = rest
                .chars()
                .next()
                .filter(|quote| *quote == '\'' || *quote == '"')
            {
                rest = &rest[quote.len_utf8()..];
                if let Some(end) = rest.find(quote) {
                    value = rest[..end].to_string();
                    rest = &rest[end + quote.len_utf8()..];
                } else {
                    value = rest.to_string();
                    rest = "";
                }
            } else if let Some(end) = rest.find(char::is_whitespace) {
                value = rest[..end].to_string();
                rest = &rest[end..];
            } else {
                value = rest.to_string();
                rest = "";
            }
        }
        if !attribute_name.is_empty() {
            attributes.insert(attribute_name, value);
        }
    }
    (name, attributes)
}
