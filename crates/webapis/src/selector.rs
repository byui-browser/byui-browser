//! Simple selector parsing for `Document::query_selector_all`.

/// Error returned when a `query_selector_all` selector is invalid or unsupported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectorError {
    /// Human-readable explanation of the selector error.
    pub message: String,
}

impl SelectorError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for SelectorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SelectorError {}

#[derive(Debug)]
pub(crate) struct SimpleSelector {
    element_name: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
}

impl SimpleSelector {
    pub(crate) fn matches(&self, element: &html::ElementData) -> bool {
        if let Some(name) = &self.element_name
            && element.get_name() != name
        {
            return false;
        }
        if let Some(id) = &self.id
            && !element
                .get_attributes()
                .iter()
                .any(|attribute| attribute.get_name() == "id" && *attribute.get_value() == *id)
        {
            return false;
        }
        self.classes.iter().all(|class| {
            element.get_attributes().iter().any(|attribute| {
                attribute.get_name() == "class"
                    && attribute
                        .get_value()
                        .split_whitespace()
                        .any(|value| value == class)
            })
        })
    }
}

pub(crate) fn parse_selector_list(selector: &str) -> Result<Vec<SimpleSelector>, SelectorError> {
    selector
        .split(',')
        .map(str::trim)
        .map(parse_simple_selector)
        .collect()
}

fn parse_simple_selector(selector: &str) -> Result<SimpleSelector, SelectorError> {
    if selector.is_empty() || selector.chars().any(char::is_whitespace) {
        return Err(SelectorError::new("unsupported or empty selector"));
    }
    let mut parsed = SimpleSelector {
        element_name: None,
        id: None,
        classes: Vec::new(),
    };
    let mut remaining = selector;
    if let Some(first) = remaining.chars().next()
        && first != '#'
        && first != '.'
        && first != '*'
    {
        let end = remaining.find(['#', '.']).unwrap_or(remaining.len());
        let name = &remaining[..end];
        if !is_identifier(name) {
            return Err(SelectorError::new("invalid element selector"));
        }
        parsed.element_name = Some(name.to_ascii_lowercase());
        remaining = &remaining[end..];
    } else if remaining.starts_with('*') {
        remaining = &remaining[1..];
    }
    while !remaining.is_empty() {
        let marker = remaining.as_bytes()[0] as char;
        if marker != '#' && marker != '.' {
            return Err(SelectorError::new("invalid selector syntax"));
        }
        let value_end = remaining[1..]
            .find(['#', '.'])
            .map_or(remaining.len(), |index| index + 1);
        let value = &remaining[1..value_end];
        if !is_identifier(value) {
            return Err(SelectorError::new("invalid selector name"));
        }
        if marker == '#' {
            if parsed.id.replace(value.to_owned()).is_some() {
                return Err(SelectorError::new("selector has multiple IDs"));
            }
        } else {
            parsed.classes.push(value.to_owned());
        }
        remaining = &remaining[value_end..];
    }
    if parsed.element_name.is_none()
        && parsed.id.is_none()
        && parsed.classes.is_empty()
        && selector != "*"
    {
        return Err(SelectorError::new(
            "selector must contain a simple selector",
        ));
    }
    Ok(parsed)
}

fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}
