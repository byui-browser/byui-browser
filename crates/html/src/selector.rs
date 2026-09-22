use crate::{HTMLDocument, HTMLElement};

/// Basic selector queries for parsed HTML documents.
pub trait Query {
    fn query(&self, selector: &str) -> Vec<HTMLElement>;

    /// Upper-case alias matching the initial data-contract spelling.
    #[allow(non_snake_case)]
    fn Query(&self, selector: String) -> Vec<HTMLElement> {
        self.query(&selector)
    }
}

impl Query for HTMLDocument {
    fn query(&self, selector: &str) -> Vec<HTMLElement> {
        self.elements
            .iter()
            .filter(|element| matches_selector(element, selector))
            .cloned()
            .collect()
    }
}

fn matches_selector(element: &HTMLElement, selector: &str) -> bool {
    match selector.strip_prefix('#') {
        Some(id) => element
            .attributes
            .get("id")
            .is_some_and(|value| value == id),
        None => match selector.strip_prefix('.') {
            Some(class) => element
                .attributes
                .get("class")
                .is_some_and(|value| value.split_whitespace().any(|value| value == class)),
            None => element.name == selector.to_ascii_lowercase(),
        },
    }
}
