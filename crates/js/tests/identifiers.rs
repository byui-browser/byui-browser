//! Unicode identifiers and `\u` escapes in identifiers: lexing, parsing,
//! binding resolution, and rejection of malformed or escaped-keyword input.

use js::lexer::{Token, tokenize, tokenize_spanned};
use js::{JsErrorCategory, Object, Realm, Value, eval, parse};

fn ident(name: &str) -> Token {
    Token::Identifier(name.into())
}

fn invalid(message: &str) -> Token {
    Token::Invalid(message.into())
}

#[test]
fn lexes_unicode_identifier_start_and_continue_characters() {
    assert_eq!(
        tokenize("café π_2 名前 Ωmega x٣ $ö _ŧ"),
        vec![
            ident("café"),
            ident("π_2"),
            ident("名前"),
            ident("Ωmega"),
            ident("x٣"),
            ident("$ö"),
            ident("_ŧ"),
        ]
    );
}

#[test]
fn combining_marks_zwnj_and_zwj_may_continue_an_identifier() {
    // `e` + U+0301 COMBINING ACUTE ACCENT, and the joiners used in Persian
    // and Indic scripts.
    assert_eq!(
        tokenize("e\u{301} a\u{200C}b c\u{200D}d"),
        vec![ident("e\u{301}"), ident("a\u{200C}b"), ident("c\u{200D}d")]
    );
}

#[test]
fn characters_that_cannot_start_an_identifier_stay_unknown() {
    assert_eq!(
        tokenize("\u{301}a"),
        vec![Token::Unknown('\u{301}'), ident("a")]
    );
    assert_eq!(tokenize("\u{200C}"), vec![Token::Unknown('\u{200C}')]);
    assert_eq!(tokenize("😀"), vec![Token::Unknown('😀')]);
    assert_eq!(tokenize("€x"), vec![Token::Unknown('€'), ident("x")]);
    // A digit is a valid continue character but never a start.
    assert_eq!(tokenize("٣x"), vec![Token::Unknown('٣'), ident("x")]);
}

#[test]
fn identifiers_end_at_characters_that_cannot_continue_them() {
    assert_eq!(
        tokenize("a€b é😀"),
        vec![
            ident("a"),
            Token::Unknown('€'),
            ident("b"),
            ident("é"),
            Token::Unknown('😀'),
        ]
    );
}

#[test]
fn decodes_unicode_escapes_in_identifiers() {
    assert_eq!(
        tokenize(r"\u0061 b\u0063 \u{64}e caf\u{E9} \u{1D400} \u0024x _\u200C"),
        vec![
            ident("a"),
            ident("bc"),
            ident("de"),
            ident("café"),
            ident("\u{1D400}"),
            ident("$x"),
            ident("_\u{200C}"),
        ]
    );
}

#[test]
fn escaped_and_raw_spellings_produce_the_same_identifier() {
    assert_eq!(tokenize(r"caf\u00e9"), tokenize("café"));
    assert_eq!(tokenize(r"\u540D\u524D"), tokenize("名前"));
    assert_eq!(tokenize(r"\u{0000000061}"), tokenize("a"));
}

#[test]
fn escaped_keywords_are_a_distinct_token() {
    assert_eq!(
        tokenize(r"\u0069f v\u{61}r nul\u006C \u0075ndefined"),
        vec![
            Token::EscapedKeyword("if".into()),
            Token::EscapedKeyword("var".into()),
            Token::EscapedKeyword("null".into()),
            Token::EscapedKeyword("undefined".into()),
        ]
    );
    // Escapes that only spell part of a longer identifier are fine.
    assert_eq!(tokenize(r"\u0069fx"), vec![ident("ifx")]);
}

#[test]
fn malformed_escapes_make_the_identifier_invalid() {
    let malformed = r"identifier contains a malformed \u escape";
    let not_u = r"identifier contains an escape other than \u";
    for (source, message) in [
        (r"\u", malformed),
        (r"\u12", malformed),
        (r"\u12G4", malformed),
        (r"a\u{61", malformed),
        (r"a\u{}", malformed),
        (r"a\u{110000}", malformed),
        (r"a\u{FFFFFFFFFFFF}b", malformed),
        (r"\x61", not_u),
        (r"a\n", not_u),
        (r"a\", not_u),
        (r"\", not_u),
    ] {
        assert_eq!(tokenize(source), vec![invalid(message)], "{source}");
    }
}

#[test]
fn escapes_for_characters_not_allowed_at_their_position_are_invalid() {
    for (source, message) in [
        (
            r"\u0031a",
            "escaped character U+0031 cannot start an identifier",
        ),
        (
            r"\u0301",
            "escaped character U+0301 cannot start an identifier",
        ),
        (
            r"\u200C",
            "escaped character U+200C cannot start an identifier",
        ),
        (
            r"a\u002Db",
            "escaped character U+002D cannot continue an identifier",
        ),
        (
            r"a\u0020b",
            "escaped character U+0020 cannot continue an identifier",
        ),
        (
            r"a\u{1F600}",
            "escaped character U+1F600 cannot continue an identifier",
        ),
        // Surrogates are never identifier characters, even as a pair.
        (
            r"\uD835\uDC00",
            "escaped character U+D835 cannot start an identifier",
        ),
        (
            r"a\uDC00",
            "escaped character U+DC00 cannot continue an identifier",
        ),
    ] {
        assert_eq!(tokenize(source), vec![invalid(message)], "{source}");
    }
}

#[test]
fn an_invalid_identifier_is_one_token_and_lexing_resumes_after_it() {
    assert_eq!(
        tokenize(r"let \u0031abc = 1;"),
        vec![
            Token::Let,
            invalid("escaped character U+0031 cannot start an identifier"),
            Token::Assign,
            Token::Number(1.0),
            Token::Semicolon,
        ]
    );
    // The first error is reported even when later escapes are also bad.
    assert_eq!(
        tokenize(r"a\x\u0020"),
        vec![invalid(r"identifier contains an escape other than \u")]
    );
}

#[test]
fn identifier_spans_cover_the_raw_source_in_utf8_bytes() {
    let source = r"  café \u0061b名 \u{31}x;";
    let entries = tokenize_spanned(source);
    let spelled: Vec<&str> = entries
        .iter()
        .map(|entry| &source[entry.start..entry.end])
        .collect();
    assert_eq!(spelled, vec!["café", r"\u0061b名", r"\u{31}x", ";"]);
    assert_eq!((entries[0].start, entries[0].end), (2, 7));
}

#[test]
fn unicode_and_escaped_identifiers_parse_as_the_decoded_name() {
    assert_eq!(
        parse(r"let caf\u00e9 = 1; café").unwrap(),
        parse("let café = 1; café").unwrap()
    );
    assert_eq!(
        parse(r"function \u0066(\u{78}) { return x; }").unwrap(),
        parse("function f(x) { return x; }").unwrap()
    );
}

#[test]
fn escaped_keywords_are_syntax_errors_outside_property_names() {
    for source in [
        r"var \u0069f = 1;",
        r"let x = tru\u0065;",
        r"\u0076ar x = 1;",
        r"function \u0072eturn() {}",
        r"function f(\u006Cet) {}",
        r"\u{6E}ull",
    ] {
        let error = parse(source).unwrap_err();
        assert!(
            error
                .found
                .as_deref()
                .is_some_and(|found| found.contains("EscapedKeyword")),
            "{source}: {error}"
        );
    }
}

#[test]
fn invalid_identifiers_are_structured_syntax_errors() {
    let error = parse(r"let ok = 1; let \u0031x = 2;").unwrap_err();
    assert_eq!(error.token_index, 6);
    assert_eq!(error.offset, Some(16));
    assert_eq!((error.line, error.column), (Some(1), Some(17)));
    assert!(error.found.unwrap().contains("U+0031 cannot start"));

    let error = eval(r"a\u{zz}").unwrap_err();
    assert_eq!(error.category, JsErrorCategory::Syntax);
    assert!(error.message.contains("malformed"), "{error:?}");
}

#[test]
fn non_ascii_identifiers_can_be_declared_read_assigned_and_passed() {
    let source = "
        let café = 1;
        var 名前 = 'n';
        const π = 3;
        function double(ν) { return ν * 2; }
        café = café + π;
        名前 = 名前 + café;
        double(café) + 名前;
    ";
    assert_eq!(eval(source), Ok(Value::String("8n4".into())));
}

#[test]
fn escaped_identifiers_resolve_to_the_same_binding_as_their_decoded_spelling() {
    let source = r"
        let caf\u00E9 = 1;
        café = café + 1;
        caf\u{e9} = caf\u{E9} * 10;
        function \u{6D}ax(a, \u0062) { if (a > b) { return \u0061; } return b; }
        max(café, 5) + m\u0061x(1, 2);
    ";
    assert_eq!(eval(source), Ok(Value::Number(22.0)));

    // Redeclaring is caught across spellings, so both name one binding.
    assert!(eval(r"let \u0078 = 1; let x = 2;").is_err());
}

#[test]
fn escaped_names_and_escaped_keywords_work_as_property_names() {
    let object = Object::new();
    object.set("café", Value::Number(1.0));
    object.set("if", Value::Number(2.0));
    let mut realm = Realm::new();
    realm
        .register_global_value("obj", Value::Object(object.clone()))
        .unwrap();

    assert_eq!(
        realm.evaluate_script(r"\u006Fbj.caf\u00E9 + obj.\u0069f + obj['café']"),
        Ok(Value::Number(4.0))
    );
    realm
        .evaluate_script(r"obj.\u{6E}ull = 3; obj.名 = 4;")
        .unwrap();
    assert_eq!(object.get("null"), Some(Value::Number(3.0)));
    assert_eq!(object.get("名"), Some(Value::Number(4.0)));
}
