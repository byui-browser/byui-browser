use crate::{Dom, Token};

/// Splits raw markup into tokens.
///
/// Currently only handles the empty document.
pub fn tokenize(input: &str) -> Vec<Token> {
    if input.is_empty() {
        return Vec::new();
    }
    todo!("TODO(html): tokenize non-empty input: {input:?}");
}

/// Parses markup into the legacy DOM tree.
///
/// Currently only handles the empty document.
pub fn parse(input: &str) -> Dom {
    if input.is_empty() {
        return Dom::default();
    }
    todo!("TODO(html): build a tree from: {input:?}");
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
