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
fn decodes_hex_unicode_and_control_escapes() {
    assert_eq!(
        tokenize(r#"'\x41B\u{43}\u{1F600}😀\0\b\f\v\q'"#),
        vec![Token::String(
            "ABC\u{1F600}\u{1F600}\0\u{8}\u{C}\u{B}q".into()
        )]
    );
}

#[test]
fn line_continuations_are_removed_from_strings() {
    assert_eq!(
        tokenize("'a\\\nb\\\r\nc'"),
        vec![Token::String("abc".into())]
    );
}

#[test]
fn unsupported_and_malformed_escapes_are_reported_as_invalid() {
    let unsupported = "string contains an unsupported escape";
    let invalid = "string contains an invalid escape";
    for (source, message) in [
        (r"'\1'", unsupported),
        (r"'\01'", unsupported),
        (r"'\uD83D'", unsupported),
        (r"'\uDE00'", unsupported),
        (r"'\x4'", invalid),
        (r"'\xZZ'", invalid),
        (r"'\u12'", invalid),
        (r"'\u{110000}'", invalid),
        (r"'\u{}'", invalid),
    ] {
        assert_eq!(
            tokenize(source),
            vec![Token::Invalid(message.into())],
            "{source}"
        );
    }
}

#[test]
fn skips_line_and_block_comments() {
    assert_eq!(
        tokenize("1 // trailing\n/* block\n comment */ + /**/2 // end"),
        vec![Token::Number(1.0), Token::Plus, Token::Number(2.0)]
    );
    assert_eq!(
        tokenize("4 / 2"),
        vec![Token::Number(4.0), Token::Divide, Token::Number(2.0)]
    );
}

#[test]
fn unterminated_block_comment_is_invalid() {
    assert_eq!(
        tokenize("1 /* never closed"),
        vec![
            Token::Number(1.0),
            Token::Invalid("unterminated block comment".into())
        ]
    );
}

#[test]
fn lexes_remainder_bitwise_and_shift_operators() {
    assert_eq!(
        tokenize("% & | ^ ~ << >> >>> <= >= && ||"),
        vec![
            Token::Remainder,
            Token::BitAnd,
            Token::BitOr,
            Token::BitXor,
            Token::BitNot,
            Token::ShiftLeft,
            Token::ShiftRight,
            Token::UnsignedShiftRight,
            Token::LessEqual,
            Token::GreaterEqual,
            Token::AndAnd,
            Token::OrOr,
        ]
    );
}

#[test]
fn treats_javascript_whitespace_as_whitespace() {
    assert_eq!(
        tokenize("1\u{FEFF}\u{2028}\u{3000}2"),
        vec![Token::Number(1.0), Token::Number(2.0)]
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

#[test]
fn spans_use_utf8_byte_offsets_and_exclude_whitespace() {
    let source = "  'é' <= 2\n";
    let entries = js::lexer::tokenize_spanned(source);
    assert_eq!(
        entries
            .iter()
            .map(|entry| (entry.start, entry.end))
            .collect::<Vec<_>>(),
        vec![(2, 6), (7, 9), (10, 11)]
    );
    assert_eq!(&source[entries[0].start..entries[0].end], "'é'");
}
