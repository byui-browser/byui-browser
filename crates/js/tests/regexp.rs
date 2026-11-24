//! Source-to-runtime coverage for regular expressions in both execution engines.
use js::{ExecutionMode, JsErrorCategory, Realm, Value, eval_with_mode, parse};

fn check(source: &str, expected: Value) {
    for mode in [ExecutionMode::Interpreter, ExecutionMode::Bytecode] {
        assert_eq!(
            eval_with_mode(source, mode)
                .unwrap_or_else(|error| panic!("{mode:?}: {source}: {error}")),
            expected,
            "{mode:?}: {source}"
        );
    }
}
fn boolean(source: &str) {
    check(source, Value::Boolean(true));
}
fn string(source: &str, expected: &str) {
    check(source, Value::String(expected.into()));
}

#[test]
fn literals_match_escapes_classes_and_flags() {
    boolean(r"/^ab+c$/i.test('ABBC') && !/^ab+c$/.test('xabbbc')");
    boolean(r"/\d+/.test('42') && !/\d/.test('٤') && !/\w/.test('é')");
    boolean(r"/[\dA-F]+/.test('A2') && /[\s\S]/.test('\n')");
    boolean(r"/\s/.test('\uFEFF') && !/\s/.test('\u0085')");
    boolean(r"/a(?=b)/.test('ab') && /(?<=a)b/.test('ab') && /(ab)\1/.test('abab')");
    boolean(r"/^b/m.test('a\rb') && !/a.b/.test('a\rb') && /a.b/s.test('a\nb')");
    boolean(r"! /a$/.test('a\n') && /a$/m.test('a\n')");
}

#[test]
fn parser_distinguishes_regex_from_division_using_expression_context() {
    boolean(r"var a = 24 / 3 / 2; a /= 2; a === 2 && /=/.test('=')");
    boolean(r#"var r = /[/'"`{}]/; r.test('/') && /a/.test('a')"#);
    boolean(r"var a = (function() { return 8; })() / 2; a === 4");
    boolean(r"var a = {n: 8}; (a.n) / 2 === 4 && ({n: 6}).n / 2 === 3");
    boolean(
        r"var a = false; if (true) a = /x/.test('x'); if (true) /x/.test('x'); {} /x/.test('x'); a",
    );
    boolean(r"function f() { return /x/; } f().test('x')");
}

#[test]
fn fresh_literals_and_constructor_clones_have_independent_state() {
    boolean(
        r"function f() { return /a/g; } var a = f(), b = f(); a.test('a'); a !== b && b.lastIndex === 0",
    );
    boolean(
        r"var a = /a/g; a.lastIndex = 2; var b = new RegExp(a); b !== a && b.lastIndex === 0 && b.flags === 'g' && RegExp(a) === a",
    );
    boolean(r"var a = RegExp('a', 'i'); a.test('A') && a instanceof RegExp");
    string(
        r"var r = /a/ymig; r.source + ':' + r.flags + ':' + r.global + ':' + r.sticky",
        "a:gimy:true:true",
    );
    string(r"String(new RegExp())", "/(?:)/");
    string(r"RegExp('/\n').toString()", r"/\/\n/");
}

#[test]
fn exec_exposes_captures_named_groups_and_utf16_indices() {
    boolean(
        r"var m = /(?<word>a)(b)?/d.exec('😀a'); m[0] === 'a' && m[1] === 'a' && m[2] === undefined && m.index === 2 && m.input === '😀a' && m.groups.word === 'a' && m.indices[0][0] === 2 && m.indices[0][1] === 3 && m.indices[2] === undefined && m.indices.groups.word[0] === 2",
    );
    boolean(r"/z/.exec('abc') === null && /a/.exec('a').groups === undefined");
}

#[test]
fn global_sticky_and_empty_exec_update_last_index() {
    boolean(
        r"var r = /a/g; r.lastIndex = 1; var ok = r.test('ba') && r.lastIndex === 2; ok && !r.test('ba') && r.lastIndex === 0",
    );
    boolean(
        r"var r = /a/y; r.lastIndex = 1; r.test('ba') && r.lastIndex === 2 && !r.test('a') && r.lastIndex === 0",
    );
    boolean(r"var r = /a/y; !r.test('ba') && r.lastIndex === 0");
    boolean(r"var r = /a/; r.lastIndex = 99; r.test('a') && r.lastIndex === 99");
    boolean(r"var r = /(?:)/g; r.exec('a'); r.lastIndex === 0");
    boolean(r"var r = /a/g; r.lastIndex = -5; r.test('a') && r.lastIndex === 1");
    boolean(r"var r = /a/g; r.lastIndex = Infinity; !r.test('a') && r.lastIndex === 0");
}

#[test]
fn string_match_and_search_support_patterns_and_preserve_state() {
    string(
        r"var r = /a/g; r.lastIndex = 50; var m = 'aba'.match(r); String(m) + ':' + r.lastIndex",
        "a,a:0",
    );
    boolean(r"'abc'.match(/z/g) === null && 'abc'.match(/(b)/)[1] === 'b'");
    boolean(r"'abc'.match('b')[0] === 'b' && 'abc'.search('b') === 1 && 'abc'.search(/z/) === -1");
    boolean(r"var r = /a/g; r.lastIndex = 99; 'ba'.search(r) === 1 && r.lastIndex === 99");
    check(r"'ab'.match(/(?:)/g).length", Value::Number(3.0));
}

#[test]
fn replace_handles_global_captures_substitutions_and_callbacks() {
    string(r"'a1b2'.replace(/\d/g, '#')", "a#b#");
    string(r"'a1b2'.replace(/\d/, '#')", "a#b2");
    string(
        r"'ab'.replace(/(a)(b)/, '$2$1:$$:$&:$01:$12:$99')",
        "ba:$:ab:a:a2:$99",
    );
    string(r"'abc'.replace(/b/, '$`-$\'')", "aa-cc");
    string(r"'ab'.replace(/(?<x>a)(z)?/, '$<x>:$2:$<missing>')", "a::b");
    string(
        r"'a1b2'.replace(/(\d)/g, function(all, digit, index, input) { return digit + index; })",
        "a11b23",
    );
    string(r"'ab'.replace(/(?:)/g, '-')", "-a-b-");
    boolean(r"var r = /a/g; 'aaa'.replace(r, 'b'); r.lastIndex === 0");
    // Adjacent string-only behavior must continue working.
    string(r"'aaa'.replace('a', 'b')", "baa");
}

#[test]
fn split_includes_captures_respects_limits_and_handles_empty_matches() {
    string(r"String('a1b2c'.split(/(\d)/))", "a,1,b,2,c");
    string(r"String('a1b2c'.split(/(\d)/, 3))", "a,1,b");
    string(r"String('abc'.split(/(?:)/))", "a,b,c");
    string(r"String('ab'.split(/(?=b)/))", "a,b");
    boolean(
        r"''.split(/(?:)/).length === 0 && ''.split(/a/).length === 1 && 'a'.split(/a/, 0).length === 0",
    );
    boolean(r"var r = /,/gy; r.lastIndex = 99; 'a,b'.split(r).length === 2 && r.lastIndex === 99");
    boolean(r"'a'.split(/(z)?/)[0] === 'a'");
}

#[test]
fn malformed_literals_are_syntax_errors_and_constructor_errors_are_catchable() {
    for source in [
        r"/[/",
        r"/(/",
        r"/a/gg",
        r"/a/z",
        "/a\n/",
        r"/unterminated",
        r"/a/\u0067",
    ] {
        let error = parse(source).expect_err(source);
        assert!(error.offset.is_some(), "{source}");
    }
    boolean(
        r"var ok = false; try { new RegExp('['); } catch (e) { ok = e instanceof SyntaxError; } ok",
    );
    boolean(
        r"var ok = false; try { RegExp('a', 'gg'); } catch (e) { ok = e.name === 'SyntaxError'; } ok",
    );
    boolean(
        r"var test = /a/.test, ok = false; try { test('a'); } catch (e) { ok = e.name === 'TypeError'; } ok",
    );
    for mode in [ExecutionMode::Interpreter, ExecutionMode::Bytecode] {
        assert_eq!(
            eval_with_mode("/a/zz", mode).unwrap_err().category,
            JsErrorCategory::Syntax
        );
    }
}

#[test]
fn regex_survives_across_realm_evaluations() {
    let realm = Realm::new();
    realm.evaluate_script("var pattern = /a/g;").unwrap();
    assert_eq!(
        realm.evaluate_script("pattern.test('aa')").unwrap(),
        Value::Boolean(true)
    );
    assert_eq!(
        realm.evaluate_script("pattern.lastIndex").unwrap(),
        Value::Number(1.0)
    );
    assert_eq!(
        realm.evaluate_script("pattern.test('aa')").unwrap(),
        Value::Boolean(true)
    );
    assert_eq!(
        realm.evaluate_script("pattern.lastIndex").unwrap(),
        Value::Number(2.0)
    );
}

#[test]
fn regex_inside_template_substitutions_keeps_delimiters_opaque() {
    string(
        r"`${ (function() { if (true) /}/.test('}'); {} /}/.test('}'); return true; })() }`",
        "true",
    );
    string(r"`${ (function() { return 8; })() / 2 }`", "4");
    string(r"`match:${ /[}']/g.test('}') }`", "match:true");
    string(
        r"`${ /a/.test('a') ? `${ /b/.test('b') }` : 'no' }`",
        "true",
    );
}

#[test]
fn supported_unicode_and_legacy_character_classes_have_explicit_semantics() {
    boolean(r"/😀/u.test('😀') && /^.$/u.test('😀')");
    boolean(r"!/[\w]/.test('é') && !/\bé\b/.test('é') && /\Bé\B/.test('é')");
    boolean(r"/[^]/.test('\n') && !/[]/.test('a') && /[a&&b]/.test('&')");
    boolean(
        r"/\u0061/.test('a') && /\cA/.test('\u0001') && /\v/.test('\u000b') && /\0/.test('\x00')",
    );
    boolean(r"/(?<x>a)\k<x>/.test('aa')");
    boolean(r"/\a/.test('a') && !/\a/.test('\x07') && /[\B]/.test('B')");
    string(
        r"'a.b[c]'.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')",
        r"a\.b\[c\]",
    );
    for source in [r"/(?i:a)/", r"/(?>a)/", r"/a++/", r"/a/v"] {
        assert!(parse(source).is_err(), "{source}");
    }
}

#[test]
fn regex_errors_keep_source_locations_after_rescanning() {
    let source = "var a = /['`]/;\nvar b = /[}]/;\nlet c = ;";
    let error = parse(source).unwrap_err();
    assert_eq!(error.line, Some(3));
    assert_eq!(error.column, Some(9));
    assert_eq!(error.offset, source.rfind(';'));
}

#[test]
fn token_level_parser_accepts_explicit_regular_expression_tokens() {
    assert!(
        parse(r"`${ (function() { function f() {} /}/.test('}'); return true; })() }`").is_ok()
    );
    use js::lexer::Token;
    assert!(
        js::parser::parse(&[Token::RegExp {
            pattern: "a".into(),
            flags: "i".into()
        }])
        .is_ok()
    );
}

#[test]
fn expensive_backtracking_is_bounded() {
    let input = format!("{}!", "a".repeat(30));
    let source = format!(r"/^(a+)+\1$/.test('{input}')");
    for mode in [ExecutionMode::Interpreter, ExecutionMode::Bytecode] {
        let error = eval_with_mode(&source, mode).unwrap_err();
        assert_eq!(error.category, JsErrorCategory::Limit);
        assert_eq!(
            error.context.as_deref(),
            Some("regular expression matching")
        );
    }
}
