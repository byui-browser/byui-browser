//! Public integration tests for CSS tokenization.

use css::tokenizer::{TokenKind, tokenize, tokenize_with_unicode_ranges};

#[test]
fn tokenizes_identifiers_and_at_keywords() {
    let tokens = tokenize("@media screen");

    assert!(
        tokens
            .iter()
            .any(|token| { matches!(&token.kind, TokenKind::AtKeyword(name) if name == "media") })
    );
    assert!(
        tokens
            .iter()
            .any(|token| { matches!(&token.kind, TokenKind::Ident(name) if name == "screen") })
    );
}

#[test]
fn tokenizes_strings_and_urls() {
    let tokens = tokenize("content: \"hello\"; background: url(image.png)");

    assert!(
        tokens
            .iter()
            .any(|token| { matches!(&token.kind, TokenKind::String(value) if value == "hello") })
    );
    assert!(
        tokens
            .iter()
            .any(|token| { matches!(&token.kind, TokenKind::Url(value) if value == "image.png") })
    );
}

#[test]
fn tokenizes_numbers_percentages_and_dimensions() {
    let tokens = tokenize("12 50% 10px 1.5em");

    assert!(
        tokens
            .iter()
            .any(|token| { matches!(&token.kind, TokenKind::Number { .. }) })
    );
    assert!(
        tokens
            .iter()
            .any(|token| { matches!(&token.kind, TokenKind::Percentage(_)) })
    );
    assert!(
        tokens
            .iter()
            .any(|token| { matches!(&token.kind, TokenKind::Dimension { .. }) })
    );
}

#[test]
fn tokenizes_functions_and_parentheses() {
    let tokens = tokenize("calc(100% - var(--gap))");

    assert!(
        tokens
            .iter()
            .any(|token| { matches!(&token.kind, TokenKind::Function(name) if name == "calc") })
    );
    assert!(
        tokens
            .iter()
            .any(|token| { matches!(&token.kind, TokenKind::Function(name) if name == "var") })
    );
}

#[test]
fn tokenizes_hashes_and_custom_properties() {
    let tokens = tokenize("#main --custom-property");

    assert!(
        tokens.iter().any(|token| {
            matches!(&token.kind, TokenKind::Hash { value, .. } if value == "main")
        })
    );
    assert!(tokens.iter().any(|token| {
        matches!(&token.kind, TokenKind::Ident(name) if name == "--custom-property")
    }));
}

#[test]
fn handles_comments_without_fusing_neighboring_tokens() {
    let tokens = tokenize("alpha/**/beta");

    let identifiers: Vec<_> = tokens
        .iter()
        .filter_map(|token| {
            if let TokenKind::Ident(value) = &token.kind {
                Some(value.as_ref())
            } else {
                None
            }
        })
        .collect();

    assert_eq!(identifiers, ["alpha", "beta"]);
}

#[test]
fn preserves_token_source_spans() {
    let source = "color: red;";
    let tokens = tokenize(source);

    assert!(
        tokens
            .iter()
            .all(|token| { token.span.start <= token.span.end && token.span.end <= source.len() })
    );
}

#[test]
fn tokenizes_css_escape_sequences() {
    let tokens = tokenize(r"\66 oo");

    assert!(
        tokens
            .iter()
            .any(|token| { matches!(&token.kind, TokenKind::Ident(name) if name == "foo") })
    );
}

fn token_kinds(input: &str) -> Vec<TokenKind<'_>> {
    tokenize(input)
        .into_iter()
        .map(|token| token.kind)
        .collect()
}

#[test]
fn tokenizes_rule_punctuation_and_skips_comments() {
    assert_eq!(
        token_kinds("p/* x */{color:red;}"),
        vec![
            TokenKind::Ident("p".into()),
            TokenKind::OpenCurly,
            TokenKind::Ident("color".into()),
            TokenKind::Colon,
            TokenKind::Ident("red".into()),
            TokenKind::Semicolon,
            TokenKind::CloseCurly,
        ]
    );
}

#[test]
fn distinguishes_id_hashes_from_other_hashes() {
    assert_eq!(
        token_kinds("#main #123"),
        vec![
            TokenKind::Hash {
                value: "main".into(),
                is_id: true
            },
            TokenKind::Whitespace,
            TokenKind::Hash {
                value: "123".into(),
                is_id: false
            },
        ]
    );
}

#[test]
fn unescapes_identifiers_and_strings() {
    assert_eq!(
        token_kinds(r#".\31 23item "a\"b""#),
        vec![
            TokenKind::Delim('.'),
            TokenKind::Ident("123item".into()),
            TokenKind::Whitespace,
            TokenKind::String("a\"b".into()),
        ]
    );
}

#[test]
fn tokenizes_bad_urls_and_bad_strings() {
    assert_eq!(token_kinds("url(a b)"), vec![TokenKind::BadUrl]);
    assert_eq!(
        token_kinds("'a\nb'"),
        vec![
            TokenKind::BadString,
            TokenKind::Whitespace,
            TokenKind::Ident("b".into()),
            TokenKind::String("".into()),
        ]
    );
}

#[test]
fn tokenizes_cdo_cdc_and_custom_properties() {
    assert_eq!(
        token_kinds("<!-- @media --x -->"),
        vec![
            TokenKind::Cdo,
            TokenKind::Whitespace,
            TokenKind::AtKeyword("media".into()),
            TokenKind::Whitespace,
            TokenKind::Ident("--x".into()),
            TokenKind::Whitespace,
            TokenKind::Cdc,
        ]
    );
}

#[test]
fn spans_cover_source_text_and_handle_non_ascii() {
    let input = "é{a:b}";
    let tokens = tokenize(input);
    assert_eq!(tokens[0].kind, TokenKind::Ident("é".into()));
    assert_eq!(&input[tokens[0].span.clone()], "é");
    assert_eq!(tokens.last().unwrap().span.end, input.len());
}

#[test]
fn replaces_null_characters_during_preprocessing() {
    assert_eq!(token_kinds("\0"), vec![TokenKind::Ident("\u{FFFD}".into())]);
}

#[test]
fn preserves_whitespace_before_quoted_url_arguments() {
    let tokens = tokenize("url(   \"image.png\")");
    assert_eq!(tokens[0].kind, TokenKind::Function("url".into()));
    assert_eq!(tokens[1].kind, TokenKind::Whitespace);
    assert_eq!(tokens[2].kind, TokenKind::String("image.png".into()));
}

#[test]
fn unicode_ranges_are_context_sensitive() {
    let ordinary = tokenize("U+416");
    assert_eq!(ordinary.len(), 2);
    assert_eq!(ordinary[0].kind, TokenKind::Ident("U".into()));
    assert!(matches!(
        ordinary[1].kind,
        TokenKind::Number { value, .. } if value == 416.0
    ));

    let unicode_ranges = tokenize_with_unicode_ranges("U+416", true);
    assert_eq!(unicode_ranges.len(), 1);
    assert_eq!(
        unicode_ranges[0].kind,
        TokenKind::UnicodeRange {
            start: 0x416,
            end: 0x416,
        }
    );
}
