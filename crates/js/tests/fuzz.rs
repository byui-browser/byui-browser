//! Randomized (fuzz-style) tests for the lexer, parser, and runtime.
//!
//! Inputs come from a small seeded generator, so every run is reproducible.
//! A failure reports the seed and the exact input. For a longer local run:
//!
//! ```text
//! JS_FUZZ_ITERATIONS=50000 JS_FUZZ_SEED=7 cargo test -p js --test fuzz --release
//! ```
//!
//! Every input is checked for these properties:
//! - nothing panics or overflows the stack;
//! - lexer spans are ordered, non-empty, and on UTF-8 character boundaries;
//! - parse errors point inside the source with one-based line and column;
//! - parsing and evaluation give the same result every time;
//! - evaluation always finishes, because it runs under a small step limit.
//!
//! When a failure is found, add the input to [`REGRESSIONS`].

use std::panic::{self, AssertUnwindSafe};

use js::{Program, lexer, parse, parser, runtime};

/// Small step budget so generated endless loops finish quickly.
const STEP_LIMIT: u64 = 20_000;

/// Inputs that once caused, or could plausibly cause, a crash.
const REGRESSIONS: &[&str] = &[
    "",
    "/",
    "/*",
    "/* *",
    "//",
    "'",
    "'\\",
    "'\\u",
    "'\\u{",
    "'\\u{110000}'",
    "'\\uD83D\\u",
    "'\\uD83D\\uD83D'",
    "'\\x",
    "1e",
    "1e+",
    "..1",
    "1..2",
    "1 // comment",
    "a\u{2028}b",
    "\u{FEFF}",
    "é",
    "😀 = 1",
    "f()()()",
    "function f() { return f; } f()()()",
    "function f() { return f(); } f()",
    "while (true) {}",
    "let x = 1; { x; let x = 2; }",
    "1 >>> -1",
    "~'x'",
];

/// Seed programs that mutations start from, covering every supported feature.
const SEEDS: &[&str] = &[
    "let total = 10; { let adjustment = 5; total = (total + adjustment * 2) / 3; } total;",
    "function fib(n) { if (n < 2) { return n; } return fib(n - 1) + fib(n - 2); } fib(8)",
    "var i = 0; while (i < 10) { i = i + 1; } i",
    "function counter() { let c = 0; function next() { c = c + 1; return c; } return next; } counter()()",
    "'a' + 1 + true + null + undefined",
    "1 == '1' && null == undefined || 0 === -0",
    "7 % 3 | 6 & 3 ^ 1 << 2 >> 1 >>> 0",
    "~5 - -3 * !0",
    "if (1 < 2) { 'yes'; } else { 'no'; }",
    "const s = '\\x41\\u0042\\u{43}\\n\\t\\0'; s",
    "/* block */ let a = 1; // line\n a",
];

#[test]
fn known_tricky_inputs_are_handled() {
    for source in REGRESSIONS {
        check_source("regression", 0, source);
    }
}

#[test]
fn random_text_is_handled() {
    let (seed, iterations) = settings(1);
    let mut rng = Rng::new(seed);
    for _ in 0..iterations {
        let source = random_text(&mut rng);
        check_source("random text", seed, &source);
    }
}

#[test]
fn token_soup_is_handled() {
    let (seed, iterations) = settings(2);
    let mut rng = Rng::new(seed);
    for _ in 0..iterations {
        let source = token_soup(&mut rng);
        check_source("token soup", seed, &source);
    }
}

#[test]
fn mutated_programs_are_handled() {
    let (seed, iterations) = settings(3);
    let mut rng = Rng::new(seed);
    for _ in 0..iterations {
        let seed_program = *rng.pick(SEEDS);
        let source = mutate(&mut rng, seed_program);
        check_source("mutation", seed, &source);
    }
}

#[test]
fn generated_valid_programs_parse_and_evaluate() {
    let (seed, iterations) = settings(4);
    let mut rng = Rng::new(seed);
    for _ in 0..iterations {
        let source = Generator::new(&mut rng).program();
        let parsed = check_source("generated program", seed, &source);
        assert!(
            parsed,
            "generated program failed to parse (seed {seed}):\n{source}\nerror: {:?}",
            parse(&source).unwrap_err()
        );
    }
}

#[test]
fn deeply_nested_inputs_are_handled() {
    let shapes: [fn(usize) -> String; 9] = [
        |n| format!("{}1{}", "(".repeat(n), ")".repeat(n)),
        |n| format!("{}1", "!".repeat(n)),
        |n| format!("{}1", "-".repeat(n)),
        |n| format!("{}{}", "{".repeat(n), "}".repeat(n)),
        |n| format!("{}1", "a=".repeat(n)),
        |n| vec!["1"; n.max(1)].join("+"),
        |n| format!("f{}", "()".repeat(n)),
        |n| format!("{}1;", "if (1) ".repeat(n)),
        |n| format!("{}{}", "function f() { ".repeat(n), "}".repeat(n)),
    ];
    for shape in shapes {
        for depth in [10, 127, 128, 129, 511, 512, 513, 2_000] {
            check_source("deep input", 0, &shape(depth));
        }
    }
}

/// Runs every check on `source`, reporting the input if any check panics.
/// Returns whether the source parsed.
fn check_source(kind: &str, seed: u64, source: &str) -> bool {
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
        check_lexer(source);
        let program = check_parser(source);
        if let Some(program) = &program {
            check_runtime(program);
        }
        program.is_some()
    }));
    outcome.unwrap_or_else(|_| panic!("{kind} check failed (seed {seed}) on input {source:?}"))
}

fn check_lexer(source: &str) {
    let spanned = lexer::tokenize_spanned(source);
    let mut previous_end = 0;
    for entry in &spanned {
        assert!(entry.start >= previous_end, "spans overlap: {entry:?}");
        assert!(entry.start < entry.end, "empty span: {entry:?}");
        assert!(entry.end <= source.len(), "span past end: {entry:?}");
        assert!(source.is_char_boundary(entry.start) && source.is_char_boundary(entry.end));
        previous_end = entry.end;
    }

    let plain = lexer::tokenize(source);
    assert_eq!(plain.len(), spanned.len());
    for (token, entry) in plain.iter().zip(&spanned) {
        assert_eq!(token, &entry.token);
    }
}

fn check_parser(source: &str) -> Option<Program> {
    let result = parse(source);
    assert_eq!(result, parse(source), "parsing is not deterministic");

    let token_level = parser::parse_program(&lexer::tokenize(source));
    assert_eq!(token_level.is_ok(), result.is_ok());

    match result {
        Ok(program) => Some(program),
        Err(error) => {
            let offset = error.offset.expect("source-level errors carry an offset");
            assert!(offset <= source.len() && source.is_char_boundary(offset));
            assert!(error.line.is_some_and(|line| line >= 1));
            assert!(error.column.is_some_and(|column| column >= 1));
            assert!(!error.message.is_empty() && !error.context.is_empty());
            assert!(!error.to_string().is_empty());
            None
        }
    }
}

fn check_runtime(program: &Program) {
    let first = runtime::evaluate_program_with_step_limit(program, STEP_LIMIT);
    let second = runtime::evaluate_program_with_step_limit(program, STEP_LIMIT);
    // Compare debug output so that NaN results compare equal.
    assert_eq!(
        format!("{first:?}"),
        format!("{second:?}"),
        "evaluation is not deterministic"
    );
    if let Ok(value) = first {
        let _ = value.to_string();
    }
}

/// Reads the seed and iteration count, offsetting the seed per test so each
/// test explores different inputs.
fn settings(test_offset: u64) -> (u64, usize) {
    let seed = std::env::var("JS_FUZZ_SEED")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0x5EED_u64);
    let iterations = std::env::var("JS_FUZZ_ITERATIONS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(500);
    (seed.wrapping_add(test_offset), iterations)
}

/// SplitMix64: a tiny, well-distributed generator, so no crate is needed.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        value ^ (value >> 31)
    }

    /// A value in `0..bound`.
    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    fn pick<'items, T>(&mut self, items: &'items [T]) -> &'items T {
        &items[self.below(items.len())]
    }
}

const SPECIAL_CHARS: &[char] = &[
    '\\', '\'', '"', '\n', '\r', '\t', '\0', '\u{2028}', '\u{FEFF}', '\u{A0}', 'é', '😀', '{', '}',
    '(', ')', '/', '*',
];

fn random_text(rng: &mut Rng) -> String {
    let length = rng.below(64);
    (0..length)
        .map(|_| {
            if rng.chance(75) {
                char::from(b' ' + rng.below(95) as u8)
            } else {
                *rng.pick(SPECIAL_CHARS)
            }
        })
        .collect()
}

const FRAGMENTS: &[&str] = &[
    "let",
    "var",
    "const",
    "if",
    "else",
    "while",
    "function",
    "return",
    "true",
    "false",
    "null",
    "undefined",
    "a",
    "b",
    "f",
    "x1",
    "$",
    "_",
    "0",
    "1.5",
    ".5",
    "1e3",
    "1e",
    "1..2",
    "'s'",
    "\"d\"",
    "'\\x41'",
    "'\\u{1F600}'",
    "'\\uD83D'",
    "'open",
    "+",
    "-",
    "*",
    "/",
    "%",
    "=",
    "==",
    "===",
    "!",
    "!=",
    "!==",
    "<",
    "<=",
    ">",
    ">=",
    "<<",
    ">>",
    ">>>",
    "&",
    "&&",
    "|",
    "||",
    "^",
    "~",
    "(",
    ")",
    "{",
    "}",
    ",",
    ";",
    "//",
    "/*",
    "*/",
    "@",
    "#",
];

const SEPARATORS: &[&str] = &[" ", "", "\n", "\t", " /* c */ "];

fn token_soup(rng: &mut Rng) -> String {
    let mut source = String::new();
    for _ in 0..rng.below(40) {
        source.push_str(rng.pick(FRAGMENTS));
        source.push_str(rng.pick(SEPARATORS));
    }
    source
}

/// Applies a few random edits to a seed program, editing whole characters so
/// the result is always valid UTF-8.
fn mutate(rng: &mut Rng, seed_program: &str) -> String {
    let mut chars: Vec<char> = seed_program.chars().collect();
    for _ in 0..=rng.below(4) {
        let position = rng.below(chars.len() + 1);
        let span = 1 + rng.below(8);
        let end = (position + span).min(chars.len());
        match rng.below(4) {
            0 => {
                chars.drain(position..end);
            }
            1 => {
                let copy: Vec<char> = chars[position..end].to_vec();
                chars.splice(position..position, copy);
            }
            2 => {
                let fragment = rng.pick(FRAGMENTS);
                chars.splice(position..position, fragment.chars());
            }
            _ => {
                if !chars.is_empty() {
                    let other = rng.below(chars.len());
                    let index = position.min(chars.len() - 1);
                    chars.swap(index, other);
                }
            }
        }
    }
    chars.into_iter().collect()
}

/// Variables declared by [`PRELUDE`]; generated code reads and assigns them.
const VALUE_NAMES: &[&str] = &["a", "b", "c"];
/// Functions declared by [`PRELUDE`]; generated code calls and redeclares them.
const FUNCTION_NAMES: &[&str] = &["f", "g", "h"];
const PARAMS: &[&str] = &["p", "q", "r"];

/// Declares every name generated code uses, so programs get past name lookup
/// and exercise deeper runtime paths instead of failing on the first read.
const PRELUDE: &str = "var a = 1; var b = 'two'; var c = true;
function f(p, q) { return p; } function g(p) { return p + 1; } function h() { return h; }\n";

const NUMBERS: &[&str] = &["0", "1", "2.5", "1e3", "0.1", "4294967296", "1e21"];
const STRINGS: &[&str] = &["''", "'hi'", r#""x\n""#, "'😀'", r"'\u{41}'", "'10'"];
const BINARY_OPERATORS: &[&str] = &[
    "+", "-", "*", "/", "%", "<", "<=", ">", ">=", "==", "!=", "===", "!==", "&", "|", "^", "<<",
    ">>", ">>>",
];

/// Generates syntactically valid programs in the supported subset.
struct Generator<'rng> {
    rng: &'rng mut Rng,
    /// Parameters of the function being generated, readable in its body.
    params: Vec<&'static str>,
    in_function: bool,
    /// Whether statements are being generated directly in the script body,
    /// where `let`/`const` would collide with the prelude's `var`s.
    top_level: bool,
}

impl<'rng> Generator<'rng> {
    fn new(rng: &'rng mut Rng) -> Self {
        Self {
            rng,
            params: Vec::new(),
            in_function: false,
            top_level: true,
        }
    }

    fn program(&mut self) -> String {
        let count = 1 + self.rng.below(6);
        let body: Vec<String> = (0..count).map(|_| self.statement(3)).collect();
        format!("{PRELUDE}{}", body.join("\n"))
    }

    fn value_name(&mut self) -> &'static str {
        if !self.params.is_empty() && self.rng.chance(50) {
            self.params[self.rng.below(self.params.len())]
        } else {
            self.rng.pick(VALUE_NAMES)
        }
    }

    fn atom(&mut self) -> String {
        match self.rng.below(5) {
            0 => self.rng.pick(NUMBERS).to_string(),
            1 => self.rng.pick(STRINGS).to_string(),
            2 => self.rng.pick(FUNCTION_NAMES).to_string(),
            3 => self.value_name().to_owned(),
            _ => self
                .rng
                .pick(&["true", "false", "null", "undefined"])
                .to_string(),
        }
    }

    fn expression(&mut self, depth: usize) -> String {
        if depth == 0 || self.rng.chance(30) {
            return self.atom();
        }
        let depth = depth - 1;
        match self.rng.below(6) {
            0 => {
                let operator = self.rng.pick(&["-", "!", "~"]).to_string();
                format!("{operator}{}", self.expression(depth))
            }
            1 => {
                let operator = self.rng.pick(BINARY_OPERATORS).to_string();
                format!(
                    "{} {operator} {}",
                    self.expression(depth),
                    self.expression(depth)
                )
            }
            2 => {
                let operator = self.rng.pick(&["&&", "||"]).to_string();
                format!(
                    "{} {operator} {}",
                    self.expression(depth),
                    self.expression(depth)
                )
            }
            3 => format!("({})", self.expression(depth)),
            // Parenthesized so it is valid as an operand of another operator.
            4 => {
                let name = self.value_name();
                format!("({name} = {})", self.expression(depth))
            }
            _ => {
                let name = self.rng.pick(FUNCTION_NAMES).to_string();
                let arguments: Vec<String> = (0..self.rng.below(3))
                    .map(|_| self.expression(depth))
                    .collect();
                format!("{name}({})", arguments.join(", "))
            }
        }
    }

    fn block(&mut self, depth: usize) -> String {
        let was_top_level = std::mem::replace(&mut self.top_level, false);
        let statements: Vec<String> = (0..self.rng.below(4))
            .map(|_| self.statement(depth))
            .collect();
        self.top_level = was_top_level;
        format!("{{ {} }}", statements.join(" "))
    }

    fn statement(&mut self, depth: usize) -> String {
        if depth == 0 {
            return format!("{};", self.expression(2));
        }
        let depth = depth - 1;
        match self.rng.below(9) {
            0 => format!("{};", self.expression(3)),
            1 => {
                let keyword = if self.top_level {
                    "var"
                } else {
                    self.rng.pick(&["let", "const", "var"])
                };
                let name = self.value_name();
                format!("{keyword} {name} = {};", self.expression(2))
            }
            2 => {
                let keyword = if self.top_level {
                    "var"
                } else {
                    self.rng.pick(&["let", "var"])
                };
                let name = self.value_name();
                format!("{keyword} {name};")
            }
            3 => self.block(depth),
            4 => {
                let condition = self.expression(2);
                let then_branch = self.block(depth);
                if self.rng.chance(50) {
                    format!("if ({condition}) {then_branch} else {}", self.block(depth))
                } else {
                    format!("if ({condition}) {then_branch}")
                }
            }
            5 => format!("while ({}) {}", self.expression(2), self.block(depth)),
            6 => {
                let name = self.rng.pick(FUNCTION_NAMES).to_string();
                let params: Vec<&'static str> = PARAMS
                    .iter()
                    .copied()
                    .filter(|_| self.rng.chance(50))
                    .collect();
                let signature = params.join(", ");
                let outer_params = std::mem::replace(&mut self.params, params);
                let was_in_function = std::mem::replace(&mut self.in_function, true);
                let body = self.block(depth);
                self.in_function = was_in_function;
                self.params = outer_params;
                format!("function {name}({signature}) {body}")
            }
            7 if self.in_function => {
                if self.rng.chance(20) {
                    "return;".to_owned()
                } else {
                    format!("return {};", self.expression(2))
                }
            }
            // `var` is the one declaration allowed as a single-statement body.
            _ => {
                let condition = self.expression(1);
                let name = self.value_name();
                format!("if ({condition}) var {name} = {};", self.expression(1))
            }
        }
    }
}
