//! Selector specificity for the string selectors the cascade matches on.

use crate::stylesheet::{Selector, Specificity};

pub fn specificity(selector: &Selector) -> Specificity {
    let mut result = Specificity(0, 0, 0);
    let mut input = selector.0.as_str();
    let mut type_position = true;
    while !input.is_empty() {
        if type_position && input.as_bytes()[0].is_ascii_alphabetic() {
            result.2 += 1;
            input = skip_identifier(input);
            type_position = false;
        } else if let Some(rest) = input.strip_prefix('#') {
            result.0 += 1;
            input = skip_identifier(rest);
            type_position = false;
        } else if let Some(rest) = input.strip_prefix('.') {
            result.1 += 1;
            input = skip_identifier(rest);
            type_position = false;
        } else if let Some(rest) = input.strip_prefix("::") {
            result.2 += 1;
            input = skip_identifier(rest);
            type_position = false;
        } else if let Some(rest) = input.strip_prefix(':') {
            result.1 += 1;
            input = skip_identifier(rest);
            type_position = false;
        } else if input.starts_with('[') {
            result.1 += 1;
            input = input
                .get(input.find(']').unwrap_or(input.len() - 1) + 1..)
                .unwrap_or("");
            type_position = false;
        } else if matches!(
            input.as_bytes()[0] as char,
            ' ' | '\t' | '\n' | '\r' | '>' | '+' | '~'
        ) {
            input = &input[1..];
            type_position = true;
        } else {
            input = &input[1..];
            type_position = false;
        }
    }
    result
}

fn skip_identifier(input: &str) -> &str {
    let end = input
        .find(|character: char| {
            !character.is_ascii_alphanumeric() && character != '_' && character != '-'
        })
        .unwrap_or(input.len());
    &input[end..]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Selector, Specificity};

    #[test]
    fn specificity_orders_ids_classes_and_elements() {
        assert!(specificity(&Selector("#main".into())) > specificity(&Selector(".card".into())));
        assert!(specificity(&Selector(".card".into())) > specificity(&Selector("div".into())));
        assert_eq!(
            specificity(&Selector("div.card#main".into())),
            Specificity(1, 1, 1)
        );
    }
}
