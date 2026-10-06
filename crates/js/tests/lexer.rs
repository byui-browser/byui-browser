use js::lexer::{Token, tokenize};

#[test]
fn lexes_adjacent_numbers_operators_and_identifiers() {
    assert_eq!(
        tokenize("let total=12.5+count;"),
        vec![
            Token::Let,
            Token::Identifier("total".into()),
            Token::Assign,
            Token::Number(12.5),
            Token::Plus,
            Token::Identifier("count".into()),
            Token::Semicolon,
        ]
    );
}

#[test]
fn lexes_keywords_comparisons_and_unknown_characters() {
    assert_eq!(
        tokenize("if (value == true) @"),
        vec![
            Token::If,
            Token::LeftParen,
            Token::Identifier("value".into()),
            Token::EqualEqual,
            Token::True,
            Token::RightParen,
            Token::Unknown('@'),
        ]
    );
}

#[test]
fn lexes_sprint_two_keywords_and_delimiters() {
    assert_eq!(
        tokenize("var while { item, undefined }"),
        vec![
            Token::Var,
            Token::While,
            Token::LeftBrace,
            Token::Identifier("item".into()),
            Token::Comma,
            Token::Undefined,
            Token::RightBrace,
        ]
    );
}

#[test]
fn lexes_strings_and_decodes_documented_escapes() {
    assert_eq!(
        tokenize(r#"'single' "double" 'line\n tab\t return\r slash\\ quote\' \"'"#),
        vec![
            Token::String("single".into()),
            Token::String("double".into()),
            Token::String("line\n tab\t return\r slash\\ quote' \"".into()),
        ]
    );
}

#[test]
fn unsupported_string_escape_is_reported_as_invalid() {
    assert_eq!(
        tokenize(r#"'bad\q escape'"#),
        vec![Token::Invalid(
            "string contains an unsupported escape".into()
        )]
    );
}

#[test]
fn lexes_longest_matching_operators() {
    assert_eq!(
        tokenize("! != !== = == === < <= > >= && ||"),
        vec![
            Token::Bang,
            Token::BangEqual,
            Token::StrictBangEqual,
            Token::Assign,
            Token::EqualEqual,
            Token::StrictEqual,
            Token::LessThan,
            Token::LessEqual,
            Token::GreaterThan,
            Token::GreaterEqual,
            Token::AndAnd,
            Token::OrOr,
        ]
    );
}

#[test]
fn malformed_numbers_and_unterminated_strings_are_invalid_tokens() {
    assert_eq!(
        tokenize("1.2.3 4e+ 'unfinished"),
        vec![
            Token::Invalid("malformed number \"1.2.3\"".into()),
            Token::Invalid("malformed number \"4e+\"".into()),
            Token::Invalid("unterminated string literal".into()),
        ]
    );
}
