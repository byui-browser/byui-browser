pub mod dom_nodes;

fn parse_html(html : String) -> Document {
    let document_id = 0;
    let mut document = Document {
        doc_type: String::new(),
        elements: HashMap::new(),
        child_ids: Vec::new(),
    };

    let mut elements = HashMap::new();
    let mut child_ids = Vec::new();
    let mut open_elements = vec![document_id];
    let mut next_id: ElementID = 1;
    let mut position = 0;

    while position < html.len() {
        let Some(relative_start) = html[position..].find('<') else {
            add_text_node(
                &mut elements,
                &mut child_ids,
                *open_elements.last().unwrap(),
                &html[position..],
                &mut next_id,
            );
            break;
        };

        let start = position + relative_start;
        add_text_node(
            &mut elements,
            &mut child_ids,
            *open_elements.last().unwrap(),
            &html[position..start],
            &mut next_id,
        );

        // Comments are ignored as DOM nodes by this small parser.
        if html[start..].starts_with("<!--") {
            if let Some(end) = html[start + 4..].find("-->") {
                position = start + 4 + end + 3;
            } else {
                break;
            }
            continue;
        }

        let Some(end) = find_tag_end(&html, start + 1) else {
            // Treat an unmatched '<' as ordinary text.
            add_text_node(
                &mut elements,
                &mut child_ids,
                *open_elements.last().unwrap(),
                &html[start..],
                &mut next_id,
            );
            break;
        };

        let tag = html[start + 1..end].trim();
        position = end + 1;

        if tag.starts_with('!') || tag.starts_with('?') {
            if tag.len() >= 8 && tag[..7].eq_ignore_ascii_case("!doctype") {
                document.doc_type = tag[7..].trim().to_string();
            }
            continue;
        }

        if let Some(name) = tag.strip_prefix('/') {
            let closing_name = name.trim().to_ascii_lowercase();
            if let Some(index) = open_elements.iter().rposition(|id| {
                elements.get(id).is_some_and(|element| match element {
                    Element::HTMLElement(element) => element.name == closing_name,
                    _ => false,
                })
            }) {
                open_elements.truncate(index);
                if open_elements.is_empty() {
                    open_elements.push(document_id);
                }
            }
            continue;
        }

        let (name, attributes, self_closing) = parse_start_tag(tag);
        if name.is_empty() {
            continue;
        }

        let element_id = next_id;
        next_id += 1;
        let is_void = is_void_element(&name);
        elements.insert(
            element_id,
            Element::HTMLElement(HTMLElement {
                name,
                element_id,
                attributes,
                child_ids: Vec::new(),
            }),
        );
        append_child(&mut elements, &mut child_ids, *open_elements.last().unwrap(), element_id);

        if !self_closing && !is_void {
            open_elements.push(element_id);
        }
    }

    document.child_ids = child_ids;
    document.elements = elements;
    document
}

fn find_tag_end(html: &str, mut position: usize) -> Option<usize> {
    let mut quote = None;
    while position < html.len() {
        let byte = html.as_bytes()[position];
        match (quote, byte) {
            (Some(current), value) if value == current => quote = None,
            (None, b'\'' | b'"') => quote = Some(byte),
            (None, b'>') => return Some(position),
            _ => {}
        }
        position += 1;
    }
    None
}

fn parse_start_tag(tag: &str) -> (String, HashMap<String, String>, bool) {
    let mut content = tag.trim();
    let self_closing = content.ends_with('/');
    if self_closing {
        content = content[..content.len() - 1].trim_end();
    }

    let name_end = content
        .find(|character: char| character.is_whitespace())
        .unwrap_or(content.len());
    let name = content[..name_end].to_ascii_lowercase();
    let mut attributes = HashMap::new();
    let mut position = name_end;

    while position < content.len() {
        while position < content.len() && content.as_bytes()[position].is_ascii_whitespace() {
            position += 1;
        }
        if position >= content.len() {
            break;
        }

        let key_start = position;
        while position < content.len()
            && !content.as_bytes()[position].is_ascii_whitespace()
            && content.as_bytes()[position] != b'='
        {
            position += 1;
        }
        let key = content[key_start..position].to_ascii_lowercase();
        while position < content.len() && content.as_bytes()[position].is_ascii_whitespace() {
            position += 1;
        }

        let mut value = String::new();
        if position < content.len() && content.as_bytes()[position] == b'=' {
            position += 1;
            while position < content.len() && content.as_bytes()[position].is_ascii_whitespace() {
                position += 1;
            }
            if position < content.len() && matches!(content.as_bytes()[position], b'\'' | b'"') {
                let quote = content.as_bytes()[position];
                position += 1;
                let value_start = position;
                while position < content.len() && content.as_bytes()[position] != quote {
                    position += 1;
                }
                value = content[value_start..position].to_string();
                if position < content.len() {
                    position += 1;
                }
            } else {
                let value_start = position;
                while position < content.len() && !content.as_bytes()[position].is_ascii_whitespace() {
                    position += 1;
                }
                value = content[value_start..position].to_string();
            }
        }
        if !key.is_empty() {
            attributes.insert(key, value);
        }
    }

    (name, attributes, self_closing)
}

fn is_void_element(name: &str) -> bool {
    matches!(name, "area" | "base" | "br" | "col" | "embed" | "hr" | "img" | "input" | "link" | "meta" | "param" | "source" | "track" | "wbr")
}

fn add_text_node(
    elements: &mut HashMap<ElementID, Element>,
    document_children: &mut Vec<ElementID>,
    parent_id: ElementID,
    text: &str,
    next_id: &mut ElementID,
) {
    if text.is_empty() {
        return;
    }
    let element_id = *next_id;
    *next_id += 1;
    elements.insert(
        element_id,
        Element::TextElement(TextElement {
            text: text.to_string(),
            element_id,
        }),
    );
    append_child(elements, document_children, parent_id, element_id);
}

fn append_child(
    elements: &mut HashMap<ElementID, Element>,
    document_children: &mut Vec<ElementID>,
    parent_id: ElementID,
    child_id: ElementID,
) {
    if parent_id == 0 {
        document_children.push(child_id);
        return;
    }
    if let Some(Element::HTMLElement(element)) = elements.get_mut(&parent_id) {
        element.child_ids.push(child_id);
    }
}

