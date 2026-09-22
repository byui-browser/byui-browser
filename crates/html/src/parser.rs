use crate::{Attribute, ElementData, HtmlDocument, Namespace, NodeId, NodeKind, SourceSpan};

/// Parses raw HTML into an arena-backed document tree.
pub fn parse_raw_html(raw: String) -> HtmlDocument {
    let mut document = HtmlDocument::new();
    let root = document.root;
    let mut stack = Vec::<NodeId>::new();
    let mut cursor = 0;

    while let Some(relative_open) = raw[cursor..].find('<') {
        let open = cursor + relative_open;
        append_text(
            &mut document,
            current_parent(&stack, root),
            &raw[cursor..open],
            cursor,
            open,
        );
        let Some(relative_close) = raw[open..].find('>') else {
            append_text(
                &mut document,
                current_parent(&stack, root),
                &raw[open..],
                open,
                raw.len(),
            );
            cursor = raw.len();
            break;
        };
        let close = open + relative_close;
        let tag = raw[open + 1..close].trim();
        let span = Some(SourceSpan {
            start: open,
            end: close + 1,
        });
        cursor = close + 1;

        // A '<' followed by whitespace is not the start of a tag. Preserve
        // this malformed markup as text instead of dropping it as an element.
        if raw[open + 1..close]
            .chars()
            .next()
            .is_some_and(char::is_whitespace)
        {
            append_text(
                &mut document,
                current_parent(&stack, root),
                &raw[open..cursor],
                open,
                cursor,
            );
            continue;
        }

        if let Some(comment_body) = tag.strip_prefix("!--") {
            let comment = comment_body.strip_suffix("--").unwrap_or(comment_body);
            let id = document.push_node(NodeKind::Comment(comment.to_owned()), span);
            document.append_child(current_parent(&stack, root), id);
            continue;
        }
        if tag
            .get(..8)
            .is_some_and(|name| name.eq_ignore_ascii_case("!doctype"))
        {
            let id = document.push_node(
                NodeKind::Doctype {
                    name: Some(tag[8..].trim().to_ascii_lowercase()),
                    public_id: None,
                    system_id: None,
                },
                span,
            );
            document.append_child(root, id);
            continue;
        }
        if let Some(closing) = tag.strip_prefix('/') {
            let name = closing
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            if let Some(position) = stack
                .iter()
                .rposition(|id| element_name(&document, *id) == Some(name.as_str()))
            {
                stack.truncate(position);
            }
            continue;
        }
        if tag.starts_with('!') || tag.starts_with('?') {
            continue;
        }

        let self_closing = tag.ends_with('/');
        let (name, attributes) = parse_start_tag(tag.trim_end_matches('/').trim());
        if name.is_empty() {
            continue;
        }
        let namespace = match name.as_str() {
            "svg" => Namespace::Svg,
            "math" => Namespace::MathMl,
            _ if stack.iter().any(|id| {
                matches!(
                    document.node(*id).map(|node| &node.kind),
                    Some(NodeKind::Element(ElementData {
                        namespace: Namespace::Svg,
                        ..
                    }))
                )
            }) =>
            {
                Namespace::Svg
            }
            _ => Namespace::Html,
        };
        let id = document.push_node(
            NodeKind::Element(ElementData {
                name: name.clone(),
                namespace,
                attributes,
            }),
            span,
        );
        document.append_child(current_parent(&stack, root), id);
        if !self_closing && !is_void_element(&name) {
            stack.push(id);
        }
    }
    if cursor < raw.len() {
        append_text(
            &mut document,
            current_parent(&stack, root),
            &raw[cursor..],
            cursor,
            raw.len(),
        );
    }
    document
}

fn current_parent(stack: &[NodeId], root: NodeId) -> NodeId {
    stack.last().copied().unwrap_or(root)
}

fn append_text(document: &mut HtmlDocument, parent: NodeId, text: &str, start: usize, end: usize) {
    if text.is_empty() {
        return;
    }
    let id = document.push_node(
        NodeKind::Text(text.to_owned()),
        Some(SourceSpan { start, end }),
    );
    document.append_child(parent, id);
}

fn element_name(document: &HtmlDocument, id: NodeId) -> Option<&str> {
    match &document.node(id)?.kind {
        NodeKind::Element(element) => Some(&element.name),
        _ => None,
    }
}

fn parse_start_tag(tag: &str) -> (String, Vec<Attribute>) {
    let mut parts = tag.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or_default().to_ascii_lowercase();
    let mut attributes = Vec::new();
    let mut rest = parts.next().unwrap_or_default().trim();
    while !rest.is_empty() {
        rest = rest.trim_start();
        let end = rest
            .find(|character: char| character.is_whitespace() || character == '=')
            .unwrap_or(rest.len());
        let attribute_name = rest[..end].to_ascii_lowercase();
        rest = rest[end..].trim_start();
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
                    value = rest[..end].to_owned();
                    rest = &rest[end + quote.len_utf8()..];
                } else {
                    value = rest.to_owned();
                    rest = "";
                }
            } else if let Some(end) = rest.find(char::is_whitespace) {
                value = rest[..end].to_owned();
                rest = &rest[end..];
            } else {
                value = rest.to_owned();
                rest = "";
            }
        }
        if !attribute_name.is_empty() {
            attributes.push(Attribute {
                name: attribute_name,
                value,
            });
        }
    }
    (name, attributes)
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
