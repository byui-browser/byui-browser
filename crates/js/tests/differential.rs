mod common;

use common::assert_equivalent;

#[test]
fn deterministic_regressions_match() {
    for source in [
        "undefined",
        "null",
        "true",
        "false",
        "0",
        "-4",
        "'text'",
        "1 + 2 * 3",
        "-'4'",
        "~1",
        "5 % 2",
        "8 >> 1",
        "1 < 2",
        "1 === 1",
        "'a' + 2",
        "true || (1 / 0)",
        "false && (1 / 0)",
        "0 || 'fallback'",
        "1 && 'kept'",
        "1; { 2 }",
        "{ 1; 2 }",
        "1 / 0",
        "1 / 'bad'",
        "~'bad'",
    ] {
        assert_equivalent(source);
    }
}

#[test]
fn generated_expression_programs_match() {
    // Small deterministic generator; bounded depth keeps CI work predictable.
    let atoms = ["0", "1", "2", "-1", "true", "false", "null", "'3'", "''"];
    let operators = ["+", "-", "*", "/", "%", "<", "===", "&", "||", "&&"];
    let mut state = 0x5eed_u32;
    for _ in 0..256 {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let left = atoms[(state as usize) % atoms.len()];
        let operator = operators[((state >> 8) as usize) % operators.len()];
        let right = atoms[((state >> 16) as usize) % atoms.len()];
        let source = format!("({left}) {operator} ({right})");
        assert_equivalent(&source);
    }
}
