//! JavaScript audit: script loading and parsing, language syntax, and the
//! built-ins and Web APIs the page's scripts reference.

use std::collections::{BTreeMap, HashSet};

use js::Value;

use super::markup::PageMap;
use super::scan::{Tok, Token, tokenize};
use super::{Area, AuditReport, Auditor, Evidence, Finding, PageScriptEngine, PageSources, Usage};

/// The document every JavaScript probe runs in. `#probe` gives element
/// probes something to find.
fn probe_page(source: &str) -> String {
    format!(
        "<!DOCTYPE html><html><head></head><body>\
         <div id=\"probe\" class=\"a b\" data-x=\"1\">text<span>child</span></div>\
         <script>{source}</script></body></html>"
    )
}

impl<E: PageScriptEngine> Auditor<E> {
    /// Whether `source` runs in a page realm and its completion value
    /// satisfies `expect`. Cached by `source`.
    fn script_works(&self, source: &str, expect: impl FnOnce(&Value) -> bool) -> bool {
        self.cached(&format!("js:{source}"), || {
            match self.engine.evaluate_page(&probe_page(source)) {
                Ok(value) => expect(&value),
                Err(error) => {
                    log::trace!("probe `{source}` failed: {error}");
                    false
                }
            }
        })
    }

    /// Whether `expression` evaluates to something other than `undefined`.
    fn exists(&self, setup: &str, expression: &str) -> bool {
        self.script_works(&format!("{setup}{expression}"), |value| {
            *value != Value::Undefined
        })
    }
}

// ---------------------------------------------------------------------------
// Token helpers

fn prev(tokens: &[Token], i: usize) -> Option<&Token> {
    i.checked_sub(1).and_then(|j| tokens.get(j))
}

fn is_member_name(tokens: &[Token], i: usize) -> bool {
    prev(tokens, i).is_some_and(|token| token.is_punct(".") || token.is_punct("?."))
}

/// Whether `tokens[i]` is the keyword `name` (not a property name or an
/// object key).
fn keyword(tokens: &[Token], i: usize, name: &str) -> bool {
    tokens[i].is_ident(name)
        && !is_member_name(tokens, i)
        && !tokens.get(i + 1).is_some_and(|next| next.is_punct(":"))
}

/// Index of the bracket that closes the one opened at `open`.
fn closing(tokens: &[Token], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate().skip(open) {
        match token.tok {
            Tok::Punct("(" | "[" | "{") => depth += 1,
            Tok::Punct(")" | "]" | "}") => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

/// The innermost unclosed bracket before `i`, as `(index, bracket)`.
fn enclosing(tokens: &[Token], i: usize) -> Option<(usize, &'static str)> {
    let mut depth = 0usize;
    for index in (0..i).rev() {
        match tokens[index].tok {
            Tok::Punct(")" | "]" | "}") => depth += 1,
            Tok::Punct(open @ ("(" | "[" | "{")) => {
                if depth == 0 {
                    return Some((index, open));
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

/// Whether the `for (` header starting at `i` contains `pred` at depth 1.
fn for_header_has(tokens: &[Token], i: usize, pred: impl Fn(&[Token], usize) -> bool) -> bool {
    if !keyword(tokens, i, "for") {
        return false;
    }
    let open = if tokens.get(i + 1).is_some_and(|t| t.is_ident("await")) {
        i + 2
    } else {
        i + 1
    };
    if !tokens.get(open).is_some_and(|t| t.is_punct("(")) {
        return false;
    }
    let Some(close) = closing(tokens, open) else {
        return false;
    };
    let mut depth = 0;
    (open + 1..close).any(|j| {
        match tokens[j].tok {
            Tok::Punct("(" | "[" | "{") => depth += 1,
            Tok::Punct(")" | "]" | "}") => depth -= 1,
            _ => {}
        }
        depth == 0 && pred(tokens, j)
    })
}

/// Tokens after which an expression (not a statement) begins.
fn expression_position(token: Option<&Token>) -> bool {
    match token.map(|token| &token.tok) {
        Some(Tok::Punct(p)) => matches!(
            *p,
            "=" | "("
                | ","
                | ":"
                | "["
                | "!"
                | "?"
                | "||"
                | "&&"
                | "??"
                | "=>"
                | "+"
                | "-"
                | "||="
                | "&&="
                | "??="
        ),
        Some(Tok::Ident(word)) => matches!(
            word.as_str(),
            "return" | "new" | "void" | "typeof" | "yield" | "await"
        ),
        _ => false,
    }
}

const STATEMENT_KEYWORDS: &[&str] = &[
    "if",
    "for",
    "while",
    "switch",
    "catch",
    "function",
    "with",
    "return",
    "typeof",
    "do",
    "else",
    "new",
    "var",
    "let",
    "const",
    "class",
    "throw",
    "case",
    "await",
    "yield",
    "delete",
    "void",
    "in",
    "of",
    "instanceof",
    "import",
    "export",
    "super",
    "this",
];

// ---------------------------------------------------------------------------
// Syntax features

/// The completion value a syntax probe must produce.
#[derive(Clone, Copy)]
enum Expect {
    Number(f64),
    Text(&'static str),
    True,
}

impl Expect {
    fn matches(self, value: &Value) -> bool {
        match self {
            Self::Number(expected) => *value == Value::Number(expected),
            Self::Text(expected) => *value == Value::String(expected.to_owned()),
            Self::True => *value == Value::Boolean(true),
        }
    }
}

struct SyntaxFeature {
    name: &'static str,
    /// Whether the feature is used at token `i`.
    detect: fn(&[Token], usize) -> bool,
    /// A script that uses only this feature plus basics, and its result.
    probe: &'static str,
    expect: Expect,
}

const SYNTAX: &[SyntaxFeature] = &[
    SyntaxFeature {
        name: "for (;;) loops",
        detect: |t, i| for_header_has(t, i, |t, j| t[j].is_punct(";")),
        probe: "var s = 0; for (var i = 0; i < 3; i = i + 1) { s = s + i; } s",
        expect: Expect::Number(3.0),
    },
    SyntaxFeature {
        name: "for…of loops",
        detect: |t, i| for_header_has(t, i, |t, j| t[j].is_ident("of")),
        probe: "var s = 0; for (var x of [1, 2]) { s = s + x; } s",
        expect: Expect::Number(3.0),
    },
    SyntaxFeature {
        name: "for…in loops",
        detect: |t, i| {
            for_header_has(t, i, |t, j| t[j].is_ident("in"))
                && !for_header_has(t, i, |t, j| t[j].is_punct(";"))
        },
        probe: "var s = ''; for (var k in {a: 1}) { s = s + k; } s",
        expect: Expect::Text("a"),
    },
    SyntaxFeature {
        name: "do…while loops",
        detect: |t, i| keyword(t, i, "do"),
        probe: "var i = 0; do { i = i + 1; } while (i < 3); i",
        expect: Expect::Number(3.0),
    },
    SyntaxFeature {
        name: "switch statements",
        detect: |t, i| keyword(t, i, "switch") && t.get(i + 1).is_some_and(|n| n.is_punct("(")),
        probe: "var r = 0; switch (2) { case 2: r = 5; break; default: r = 1; } r",
        expect: Expect::Number(5.0),
    },
    SyntaxFeature {
        name: "Labeled statements",
        detect: |t, i| {
            t[i].ident()
                .is_some_and(|w| !STATEMENT_KEYWORDS.contains(&w) && w != "default")
                && t.get(i + 1).is_some_and(|n| n.is_punct(":"))
                && t.get(i + 2)
                    .is_some_and(|n| n.is_ident("for") || n.is_ident("while") || n.is_ident("do"))
                && prev(t, i).is_none_or(|p| p.is_punct(";") || p.is_punct("{") || p.is_punct("}"))
        },
        probe: "var i = 0; outer: while (true) { i = i + 1; break outer; } i",
        expect: Expect::Number(1.0),
    },
    SyntaxFeature {
        name: "Conditional operator (a ? b : c)",
        detect: |t, i| t[i].is_punct("?"),
        probe: "true ? 1 : 2",
        expect: Expect::Number(1.0),
    },
    SyntaxFeature {
        name: "typeof operator",
        detect: |t, i| keyword(t, i, "typeof"),
        probe: "typeof 1",
        expect: Expect::Text("number"),
    },
    SyntaxFeature {
        name: "instanceof operator",
        detect: |t, i| keyword(t, i, "instanceof"),
        probe: "function F() {} var o = new F(); o instanceof F",
        expect: Expect::True,
    },
    SyntaxFeature {
        name: "in operator",
        detect: |t, i| {
            keyword(t, i, "in")
                && !enclosing(t, i).is_some_and(|(open, _)| {
                    open > 0 && (t[open - 1].is_ident("for") || t[open - 1].is_ident("await"))
                })
        },
        probe: "'a' in {a: 1}",
        expect: Expect::True,
    },
    SyntaxFeature {
        name: "delete operator",
        detect: |t, i| keyword(t, i, "delete"),
        probe: "var o = {a: 1}; delete o.a; o.a === undefined",
        expect: Expect::True,
    },
    SyntaxFeature {
        name: "void operator",
        detect: |t, i| keyword(t, i, "void"),
        probe: "void 0 === undefined",
        expect: Expect::True,
    },
    SyntaxFeature {
        name: "Unary plus (+x)",
        detect: |t, i| {
            t[i].is_punct("+")
                && match prev(t, i).map(|p| &p.tok) {
                    None => true,
                    Some(Tok::Punct(p)) => !matches!(*p, ")" | "]" | "}" | "++" | "--"),
                    Some(Tok::Ident(w)) => {
                        matches!(w.as_str(), "return" | "typeof" | "case" | "void")
                    }
                    Some(_) => false,
                }
        },
        probe: "+'3' === 3",
        expect: Expect::True,
    },
    SyntaxFeature {
        name: "Increment and decrement (++, --)",
        detect: |t, i| t[i].is_punct("++") || t[i].is_punct("--"),
        probe: "var i = 1; i++; ++i; i",
        expect: Expect::Number(3.0),
    },
    SyntaxFeature {
        name: "Compound assignment (+=, -=, …)",
        detect: |t, i| {
            matches!(
                t[i].tok,
                Tok::Punct(
                    "+=" | "-="
                        | "*="
                        | "/="
                        | "%="
                        | "&="
                        | "|="
                        | "^="
                        | "<<="
                        | ">>="
                        | ">>>="
                        | "**="
                )
            )
        },
        probe: "var i = 1; i += 2; i",
        expect: Expect::Number(3.0),
    },
    SyntaxFeature {
        name: "Exponentiation (**)",
        detect: |t, i| t[i].is_punct("**"),
        probe: "2 ** 3",
        expect: Expect::Number(8.0),
    },
    SyntaxFeature {
        name: "Nullish coalescing (??)",
        detect: |t, i| t[i].is_punct("??"),
        probe: "null ?? 2",
        expect: Expect::Number(2.0),
    },
    SyntaxFeature {
        name: "Logical assignment (&&=, ||=, ??=)",
        detect: |t, i| matches!(t[i].tok, Tok::Punct("&&=" | "||=" | "??=")),
        probe: "var a = null; a ??= 2; a",
        expect: Expect::Number(2.0),
    },
    SyntaxFeature {
        name: "Optional chaining (?.)",
        detect: |t, i| t[i].is_punct("?."),
        probe: "var o = null; o?.a === undefined",
        expect: Expect::True,
    },
    SyntaxFeature {
        name: "Multiple declarators (var a = 1, b = 2)",
        detect: |t, i| {
            if !(keyword(t, i, "var") || keyword(t, i, "let") || keyword(t, i, "const")) {
                return false;
            }
            let mut depth = 0i32;
            for token in &t[i + 1..] {
                match &token.tok {
                    Tok::Punct("(" | "[" | "{") => depth += 1,
                    Tok::Punct(")" | "]" | "}") => {
                        depth -= 1;
                        if depth < 0 {
                            return false;
                        }
                    }
                    Tok::Punct(";") if depth == 0 => return false,
                    Tok::Punct(",") if depth == 0 => return true,
                    Tok::Ident(w)
                        if depth == 0
                            && matches!(
                                w.as_str(),
                                "in" | "of"
                                    | "var"
                                    | "let"
                                    | "const"
                                    | "function"
                                    | "if"
                                    | "return"
                                    | "for"
                                    | "while"
                                    | "class"
                            ) =>
                    {
                        return false;
                    }
                    _ => {}
                }
            }
            false
        },
        probe: "var a = 1, b = 2; a + b",
        expect: Expect::Number(3.0),
    },
    SyntaxFeature {
        name: "Destructuring",
        detect: |t, i| {
            (keyword(t, i, "var") || keyword(t, i, "let") || keyword(t, i, "const"))
                && t.get(i + 1)
                    .is_some_and(|n| n.is_punct("{") || n.is_punct("["))
        },
        probe: "var {a} = {a: 1}; var [b] = [2]; a + b",
        expect: Expect::Number(3.0),
    },
    SyntaxFeature {
        name: "Arrow functions (=>)",
        detect: |t, i| t[i].is_punct("=>"),
        probe: "var f = (a) => a + 1; f(1)",
        expect: Expect::Number(2.0),
    },
    SyntaxFeature {
        name: "Function expressions",
        detect: |t, i| {
            if !keyword(t, i, "function") {
                return false;
            }
            let before = match prev(t, i) {
                Some(p) if p.is_ident("async") => prev(t, i - 1),
                other => other,
            };
            expression_position(before)
        },
        probe: "var f = function (a) { return a + 1; }; f(1)",
        expect: Expect::Number(2.0),
    },
    SyntaxFeature {
        name: "Default parameter values",
        detect: |t, i| {
            let open = if keyword(t, i, "function") {
                let mut j = i + 1;
                while t.get(j).is_some_and(|n| !n.is_punct("(")) && j < i + 3 {
                    j += 1;
                }
                j
            } else if t[i].is_punct("=>") && prev(t, i).is_some_and(|p| p.is_punct(")")) {
                match enclosing(t, i - 1) {
                    Some((open, "(")) => open,
                    _ => return false,
                }
            } else {
                return false;
            };
            if !t.get(open).is_some_and(|n| n.is_punct("(")) {
                return false;
            }
            let Some(close) = closing(t, open) else {
                return false;
            };
            let mut depth = 0;
            (open + 1..close).any(|j| {
                match t[j].tok {
                    Tok::Punct("(" | "[" | "{") => depth += 1,
                    Tok::Punct(")" | "]" | "}") => depth -= 1,
                    _ => {}
                }
                depth == 0 && t[j].is_punct("=")
            })
        },
        probe: "function f(a = 2) { return a; } f()",
        expect: Expect::Number(2.0),
    },
    SyntaxFeature {
        name: "Spread and rest in calls and arrays (...)",
        detect: |t, i| t[i].is_punct("...") && !matches!(enclosing(t, i), Some((_, "{"))),
        probe: "function f(...xs) { return xs.length; } var a = [1, 2]; f(...a, ...[3])",
        expect: Expect::Number(3.0),
    },
    SyntaxFeature {
        name: "Object spread and rest ({...obj})",
        detect: |t, i| t[i].is_punct("...") && matches!(enclosing(t, i), Some((_, "{"))),
        probe: "var a = {x: 1}; var b = {...a}; b.x",
        expect: Expect::Number(1.0),
    },
    SyntaxFeature {
        name: "Object literal methods ({ m() {} })",
        detect: |t, i| {
            t[i].ident()
                .is_some_and(|w| !STATEMENT_KEYWORDS.contains(&w))
                && prev(t, i).is_some_and(|p| p.is_punct("{") || p.is_punct(","))
                && t.get(i + 1).is_some_and(|n| n.is_punct("("))
                && closing(t, i + 1)
                    .is_some_and(|close| t.get(close + 1).is_some_and(|n| n.is_punct("{")))
                && matches!(enclosing(t, i), Some((open, "{")) if expression_position(prev(t, open)))
        },
        probe: "var o = { m() { return 1; } }; o.m()",
        expect: Expect::Number(1.0),
    },
    SyntaxFeature {
        name: "Getters and setters",
        detect: |t, i| {
            (keyword(t, i, "get") || keyword(t, i, "set"))
                && prev(t, i).is_some_and(|p| {
                    p.is_punct("{")
                        || p.is_punct(",")
                        || p.is_punct("}")
                        || p.is_punct(";")
                        || p.is_ident("static")
                })
                && t.get(i + 1).is_some_and(|n| {
                    matches!(n.tok, Tok::Ident(_) | Tok::Str { .. } | Tok::Private)
                        || n.is_punct("[")
                })
                && t.get(i + 2).is_some_and(|n| n.is_punct("("))
        },
        probe: "var o = { get x() { return 1; } }; o.x",
        expect: Expect::Number(1.0),
    },
    SyntaxFeature {
        name: "Classes",
        detect: |t, i| keyword(t, i, "class") && !expression_position(prev(t, i)),
        probe: "class A { constructor() { this.x = 1; } m() { return this.x; } } var o = new A(); o.m()",
        expect: Expect::Number(1.0),
    },
    SyntaxFeature {
        name: "Class expressions",
        detect: |t, i| keyword(t, i, "class") && expression_position(prev(t, i)),
        probe: "var A = class { m() { return 1; } }; var o = new A(); o.m()",
        expect: Expect::Number(1.0),
    },
    SyntaxFeature {
        name: "Static class members",
        detect: |t, i| {
            keyword(t, i, "static")
                && prev(t, i).is_some_and(|p| p.is_punct("{") || p.is_punct("}") || p.is_punct(";"))
        },
        probe: "class A { static m() { return 1; } } A.m()",
        expect: Expect::Number(1.0),
    },
    SyntaxFeature {
        name: "Private class members (#name)",
        detect: |t, i| t[i].tok == Tok::Private,
        probe: "class A { #x = 1; g() { return this.#x; } } var o = new A(); o.g()",
        expect: Expect::Number(1.0),
    },
    SyntaxFeature {
        name: "Template literals",
        detect: |t, i| {
            t[i].tok == Tok::Template
                && !prev(t, i).is_some_and(|p| {
                    matches!(p.tok, Tok::Ident(_)) && !expression_position(Some(p))
                        || p.is_punct(")")
                        || p.is_punct("]")
                })
        },
        probe: "var n = 2; `a${n}b`",
        expect: Expect::Text("a2b"),
    },
    SyntaxFeature {
        name: "Tagged templates",
        detect: |t, i| {
            t[i].tok == Tok::Template
                && prev(t, i).is_some_and(|p| {
                    matches!(p.tok, Tok::Ident(_)) && !expression_position(Some(p))
                        || p.is_punct(")")
                        || p.is_punct("]")
                })
        },
        probe: "function t(s) { return s[0]; } t`x`",
        expect: Expect::Text("x"),
    },
    SyntaxFeature {
        name: "Regular expression literals",
        detect: |t, i| t[i].tok == Tok::Regex,
        probe: "var r = /a+/g; 1",
        expect: Expect::Number(1.0),
    },
    SyntaxFeature {
        name: "String escape sequences (\\x, \\u, \\n)",
        detect: |t, i| t[i].tok == Tok::Str { escapes: true },
        probe: "'\\x41\\u0042' === 'AB'",
        expect: Expect::True,
    },
    SyntaxFeature {
        name: "Hex, binary, and octal literals",
        detect: |t, i| matches!(&t[i].tok, Tok::Num(n) if n.len() > 1 && n.starts_with('0') && n[1..2].chars().all(|c| "xXbBoO".contains(c))),
        probe: "0xff + 0b1 + 0o7",
        expect: Expect::Number(263.0),
    },
    SyntaxFeature {
        name: "Exponent number literals (1e3)",
        detect: |t, i| matches!(&t[i].tok, Tok::Num(n) if !n.starts_with("0x") && !n.starts_with("0X") && n.contains(['e', 'E'])),
        probe: "1e3",
        expect: Expect::Number(1000.0),
    },
    SyntaxFeature {
        name: "Numeric separators (1_000)",
        detect: |t, i| matches!(&t[i].tok, Tok::Num(n) if n.contains('_')),
        probe: "1_000",
        expect: Expect::Number(1000.0),
    },
    SyntaxFeature {
        name: "BigInt literals (10n)",
        detect: |t, i| matches!(&t[i].tok, Tok::Num(n) if n.ends_with('n')),
        probe: "var x = 10n; 1",
        expect: Expect::Number(1.0),
    },
    SyntaxFeature {
        name: "async functions",
        detect: |t, i| {
            keyword(t, i, "async")
                && t.get(i + 1).is_some_and(|n| {
                    n.is_ident("function") || n.is_punct("(") || matches!(n.tok, Tok::Ident(_))
                })
        },
        probe: "async function f() { return 1; } f(); 1",
        expect: Expect::Number(1.0),
    },
    SyntaxFeature {
        name: "await",
        detect: |t, i| keyword(t, i, "await"),
        probe: "async function f() { var v = await 1; return v; } f(); 1",
        expect: Expect::Number(1.0),
    },
    SyntaxFeature {
        name: "Generators (function*, yield)",
        detect: |t, i| {
            (keyword(t, i, "function") && t.get(i + 1).is_some_and(|n| n.is_punct("*")))
                || keyword(t, i, "yield")
        },
        probe: "function* g() { yield 1; } var it = g(); it.next().value",
        expect: Expect::Number(1.0),
    },
    SyntaxFeature {
        name: "arguments object",
        detect: |t, i| keyword(t, i, "arguments"),
        probe: "function f() { return arguments.length; } f(1, 2)",
        expect: Expect::Number(2.0),
    },
    SyntaxFeature {
        name: "debugger statement",
        detect: |t, i| keyword(t, i, "debugger"),
        probe: "debugger; 1",
        expect: Expect::Number(1.0),
    },
    SyntaxFeature {
        name: "ES module syntax (import / export)",
        detect: |t, i| {
            (keyword(t, i, "import")
                && t.get(i + 1)
                    .is_some_and(|n| !n.is_punct("(") && !n.is_punct("."))
                && prev(t, i).is_none_or(|p| p.is_punct(";") || p.is_punct("}")))
                || (keyword(t, i, "export")
                    && prev(t, i).is_none_or(|p| p.is_punct(";") || p.is_punct("}")))
        },
        probe: "import x from './probe.js'; 1",
        expect: Expect::Number(1.0),
    },
    SyntaxFeature {
        name: "Dynamic import()",
        detect: |t, i| keyword(t, i, "import") && t.get(i + 1).is_some_and(|n| n.is_punct("(")),
        probe: "var p = import('./probe.js'); 1",
        expect: Expect::Number(1.0),
    },
];

// ---------------------------------------------------------------------------
// APIs

/// Globals that pages commonly use. Only these names are checked as global
/// references, so a page's own variables are never reported.
const GLOBALS: &[&str] = &[
    // Language built-ins.
    "Array",
    "ArrayBuffer",
    "Atomics",
    "BigInt",
    "Boolean",
    "DataView",
    "Date",
    "Error",
    "EvalError",
    "FinalizationRegistry",
    "Float32Array",
    "Float64Array",
    "Function",
    "Infinity",
    "Int16Array",
    "Int32Array",
    "Int8Array",
    "Intl",
    "JSON",
    "Map",
    "Math",
    "NaN",
    "Number",
    "Object",
    "Promise",
    "Proxy",
    "RangeError",
    "ReferenceError",
    "Reflect",
    "RegExp",
    "Set",
    "SharedArrayBuffer",
    "String",
    "Symbol",
    "SyntaxError",
    "TypeError",
    "URIError",
    "Uint16Array",
    "Uint32Array",
    "Uint8Array",
    "Uint8ClampedArray",
    "WeakMap",
    "WeakRef",
    "WeakSet",
    "decodeURI",
    "decodeURIComponent",
    "encodeURI",
    "encodeURIComponent",
    "escape",
    "eval",
    "globalThis",
    "isFinite",
    "isNaN",
    "parseFloat",
    "parseInt",
    "unescape",
    // Window and document.
    "window",
    "self",
    "document",
    "location",
    "history",
    "navigator",
    "screen",
    "frames",
    "parent",
    "top",
    "opener",
    "visualViewport",
    "console",
    "alert",
    "confirm",
    "prompt",
    "print",
    "open",
    "close",
    "focus",
    "blur",
    "scrollTo",
    "scrollBy",
    "scroll",
    "scrollX",
    "scrollY",
    "pageXOffset",
    "pageYOffset",
    "innerWidth",
    "innerHeight",
    "outerWidth",
    "outerHeight",
    "devicePixelRatio",
    "getComputedStyle",
    "getSelection",
    "matchMedia",
    "addEventListener",
    "removeEventListener",
    "dispatchEvent",
    "postMessage",
    // Timers and scheduling.
    "setTimeout",
    "clearTimeout",
    "setInterval",
    "clearInterval",
    "requestAnimationFrame",
    "cancelAnimationFrame",
    "requestIdleCallback",
    "cancelIdleCallback",
    "queueMicrotask",
    "structuredClone",
    "reportError",
    // Storage, network, and data.
    "localStorage",
    "sessionStorage",
    "indexedDB",
    "caches",
    "fetch",
    "XMLHttpRequest",
    "WebSocket",
    "EventSource",
    "Headers",
    "Request",
    "Response",
    "FormData",
    "URL",
    "URLSearchParams",
    "Blob",
    "File",
    "FileReader",
    "TextEncoder",
    "TextDecoder",
    "atob",
    "btoa",
    "crypto",
    "performance",
    "AbortController",
    "AbortSignal",
    "BroadcastChannel",
    "MessageChannel",
    "Worker",
    "SharedWorker",
    "ServiceWorker",
    "Notification",
    "Image",
    "Audio",
    "Option",
    // DOM interfaces.
    "Node",
    "Element",
    "HTMLElement",
    "Document",
    "DocumentFragment",
    "Text",
    "Comment",
    "Event",
    "EventTarget",
    "CustomEvent",
    "MouseEvent",
    "KeyboardEvent",
    "PointerEvent",
    "TouchEvent",
    "FocusEvent",
    "InputEvent",
    "WheelEvent",
    "ErrorEvent",
    "MessageEvent",
    "DOMParser",
    "XMLSerializer",
    "MutationObserver",
    "IntersectionObserver",
    "ResizeObserver",
    "PerformanceObserver",
    "NodeList",
    "HTMLCollection",
    "DOMRect",
    "Range",
    "Selection",
    "ShadowRoot",
    "customElements",
    "CSS",
    "CSSStyleSheet",
    "HTMLCanvasElement",
    "CanvasRenderingContext2D",
    "Path2D",
    "OffscreenCanvas",
    "ImageBitmap",
    "createImageBitmap",
    "WebGLRenderingContext",
    "AudioContext",
    "MediaQueryList",
    "DOMException",
    "Animation",
    "KeyframeEffect",
    "ClipboardItem",
];

/// Globals whose members are checked when a page reads `Global.member`.
const NAMESPACES: &[&str] = &[
    "document",
    "window",
    "navigator",
    "location",
    "history",
    "console",
    "Math",
    "JSON",
    "Object",
    "Array",
    "Number",
    "String",
    "Promise",
    "Reflect",
    "Date",
    "performance",
    "screen",
    "localStorage",
    "sessionStorage",
    "customElements",
    "Intl",
    "Symbol",
    "URL",
    "CSS",
    "crypto",
    "Element",
    "Node",
    "HTMLElement",
    "Event",
    "Function",
];

/// How to obtain an instance of an interface inside a probe script.
#[derive(Clone, Copy)]
struct Receiver {
    interface: &'static str,
    setup: &'static str,
    expression: &'static str,
}

const ELEMENT: Receiver = Receiver {
    interface: "Element",
    setup: "",
    expression: "document.getElementById('probe')",
};
const EVENT: Receiver = Receiver {
    interface: "Event",
    setup: "",
    expression: "new Event('probe')",
};
const MOUSE_EVENT: Receiver = Receiver {
    interface: "MouseEvent",
    setup: "",
    expression: "new MouseEvent('click')",
};
const KEYBOARD_EVENT: Receiver = Receiver {
    interface: "KeyboardEvent",
    setup: "",
    expression: "new KeyboardEvent('keydown')",
};
const ARRAY: Receiver = Receiver {
    interface: "Array",
    setup: "",
    expression: "[1, 2]",
};
const STRING: Receiver = Receiver {
    interface: "String",
    setup: "var probeString = 'ab'; ",
    expression: "probeString",
};
const NUMBER: Receiver = Receiver {
    interface: "Number",
    setup: "var probeNumber = 1.5; ",
    expression: "probeNumber",
};
const FUNCTION: Receiver = Receiver {
    interface: "Function",
    setup: "function probeFunction() {} ",
    expression: "probeFunction",
};
const PROMISE: Receiver = Receiver {
    interface: "Promise",
    setup: "",
    expression: "Promise.resolve(1)",
};
const REGEXP: Receiver = Receiver {
    interface: "RegExp",
    setup: "var probeRegExp = new RegExp('a'); ",
    expression: "probeRegExp",
};
const DATE: Receiver = Receiver {
    interface: "Date",
    setup: "",
    expression: "new Date(0)",
};
const OBJECT: Receiver = Receiver {
    interface: "Object",
    setup: "var probeObject = {}; ",
    expression: "probeObject",
};
const CLASS_LIST: Receiver = Receiver {
    interface: "DOMTokenList (classList)",
    setup: "",
    expression: "document.getElementById('probe').classList",
};
const STYLE: Receiver = Receiver {
    interface: "CSSStyleDeclaration (style)",
    setup: "",
    expression: "document.getElementById('probe').style",
};

/// Instance members recognized by name, and the interfaces that have them.
fn instance_receivers(name: &str) -> &'static [Receiver] {
    match name {
        "classList"
        | "className"
        | "getAttribute"
        | "setAttribute"
        | "removeAttribute"
        | "hasAttribute"
        | "toggleAttribute"
        | "appendChild"
        | "removeChild"
        | "insertBefore"
        | "replaceChild"
        | "cloneNode"
        | "closest"
        | "matches"
        | "getBoundingClientRect"
        | "getClientRects"
        | "scrollIntoView"
        | "insertAdjacentHTML"
        | "insertAdjacentElement"
        | "insertAdjacentText"
        | "innerHTML"
        | "outerHTML"
        | "textContent"
        | "innerText"
        | "dataset"
        | "children"
        | "childNodes"
        | "parentNode"
        | "parentElement"
        | "nextElementSibling"
        | "previousElementSibling"
        | "firstElementChild"
        | "lastElementChild"
        | "firstChild"
        | "lastChild"
        | "nextSibling"
        | "previousSibling"
        | "append"
        | "prepend"
        | "replaceWith"
        | "replaceChildren"
        | "before"
        | "after"
        | "attachShadow"
        | "shadowRoot"
        | "offsetWidth"
        | "offsetHeight"
        | "offsetTop"
        | "offsetLeft"
        | "offsetParent"
        | "clientWidth"
        | "clientHeight"
        | "scrollTop"
        | "scrollLeft"
        | "scrollHeight"
        | "scrollWidth"
        | "getElementsByClassName"
        | "getElementsByTagName"
        | "tagName"
        | "nodeType"
        | "nodeName"
        | "nodeValue"
        | "hasChildNodes"
        | "contains"
        | "animate"
        | "requestFullscreen"
        | "isConnected"
        | "ownerDocument"
        | "getRootNode"
        | "compareDocumentPosition"
        | "attributes"
        | "querySelector"
        | "querySelectorAll"
        | "addEventListener"
        | "removeEventListener"
        | "dispatchEvent"
        | "click"
        | "focus"
        | "blur"
        | "style"
        | "tabIndex"
        | "setPointerCapture"
        | "releasePointerCapture"
        | "getAnimations" => &[ELEMENT],
        "preventDefault"
        | "stopPropagation"
        | "stopImmediatePropagation"
        | "currentTarget"
        | "defaultPrevented"
        | "composedPath"
        | "isTrusted"
        | "timeStamp" => &[EVENT],
        "clientX" | "clientY" | "pageX" | "pageY" | "screenX" | "screenY" | "offsetX"
        | "offsetY" | "button" | "buttons" | "relatedTarget" | "ctrlKey" | "shiftKey"
        | "altKey" | "metaKey" => &[MOUSE_EVENT],
        "keyCode" | "charCode" | "isComposing" => &[KEYBOARD_EVENT],
        "forEach" | "map" | "filter" | "reduce" | "reduceRight" | "find" | "findIndex"
        | "findLast" | "findLastIndex" | "some" | "every" | "join" | "push" | "pop" | "shift"
        | "unshift" | "splice" | "sort" | "reverse" | "flat" | "flatMap" | "fill"
        | "copyWithin" | "toSorted" | "toReversed" | "toSpliced" => &[ARRAY],
        "includes" | "indexOf" | "lastIndexOf" | "slice" | "concat" | "at" => &[ARRAY, STRING],
        "split" | "trim" | "trimStart" | "trimEnd" | "toUpperCase" | "toLowerCase"
        | "startsWith" | "endsWith" | "padStart" | "padEnd" | "repeat" | "charAt"
        | "charCodeAt" | "codePointAt" | "substring" | "substr" | "match" | "matchAll"
        | "replace" | "replaceAll" | "localeCompare" | "normalize" | "search" => &[STRING],
        "toFixed" | "toPrecision" => &[NUMBER],
        "bind" | "call" | "apply" => &[FUNCTION],
        "then" | "catch" | "finally" => &[PROMISE],
        "test" | "exec" => &[REGEXP],
        "hasOwnProperty" | "propertyIsEnumerable" | "isPrototypeOf" => &[OBJECT],
        "getTime" | "getFullYear" | "getMonth" | "getDate" | "getDay" | "getHours"
        | "getMinutes" | "getSeconds" | "getMilliseconds" | "getTimezoneOffset" | "toISOString"
        | "toLocaleDateString" | "toLocaleTimeString" | "setFullYear" | "setMonth" | "setDate"
        | "setHours" => &[DATE],
        _ => &[],
    }
}

/// Members of a sub-object reached by a known path, such as
/// `el.classList.add`.
fn path_receiver(parent: &str, name: &str) -> Option<Receiver> {
    match (parent, name) {
        ("classList", "add" | "remove" | "toggle" | "contains" | "replace" | "item") => {
            Some(CLASS_LIST)
        }
        ("style", "setProperty" | "removeProperty" | "getPropertyValue" | "cssText") => Some(STYLE),
        _ => None,
    }
}

/// Where one token of one script is, for the report.
struct ScriptTokens {
    label: String,
    tokens: Vec<Token>,
}

impl ScriptTokens {
    fn location(&self, i: usize) -> String {
        format!("{}:{}", self.label, self.tokens[i].line)
    }
}

pub(crate) fn audit<E: PageScriptEngine>(
    auditor: &Auditor<E>,
    page: &PageSources<'_>,
    map: &PageMap,
    script_error: Option<&str>,
    report: &mut AuditReport,
) {
    audit_execution(page, map, script_error, report);

    // Only scripts the browser runs as JavaScript are scanned for features.
    let scripts: Vec<ScriptTokens> =
        page.scripts
            .iter()
            .enumerate()
            .filter(|(index, source)| {
                !source.trim().is_empty()
                    && map.scripts.get(*index).is_none_or(|slot| {
                        slot.is_classic() || slot.kind.as_deref() == Some("module")
                    })
            })
            .map(|(index, source)| ScriptTokens {
                label: map.script_label(index),
                tokens: tokenize(source),
            })
            .collect();

    // Engine parse failures are reported with the feature that likely caused them.
    for (index, source) in page.scripts.iter().enumerate() {
        // Data blocks are already reported by `audit_execution`.
        let data_block = map
            .scripts
            .get(index)
            .is_some_and(|slot| !slot.is_classic() && slot.kind.as_deref() != Some("module"));
        if source.trim().is_empty() || data_block {
            continue;
        }
        let label = map.script_label(index);
        let outcome = std::panic::catch_unwind(|| js::parse(source).err());
        let error = match outcome {
            Ok(None) => continue,
            Ok(Some(error)) => {
                let at = error
                    .line
                    .map(|line| format!(" at line {line}"))
                    .unwrap_or_default();
                format!("The engine stops parsing{at}: {}.", error.message)
            }
            Err(_) => "The engine's parser crashed on this script.".to_owned(),
        };
        let mut finding = Finding::new(
            Area::ScriptExecution,
            format!("{label} cannot be parsed"),
            format!("{error} None of this script runs."),
            Evidence::Probed,
        );
        finding.location = Some(label);
        report.findings.push(finding);
    }

    audit_syntax(auditor, &scripts, report);
    audit_apis(auditor, &scripts, report);
}

fn audit_execution(
    page: &PageSources<'_>,
    map: &PageMap,
    script_error: Option<&str>,
    report: &mut AuditReport,
) {
    if let Some(error) = script_error {
        report.findings.push(Finding::new(
            Area::ScriptExecution,
            "Page scripts stopped with an error",
            format!(
                "{error}. The browser stops at the first failing script, so later scripts and \
                 event listeners they would register never run."
            ),
            Evidence::Observed,
        ));
    }
    for (index, slot) in map.scripts.iter().enumerate() {
        let source = page
            .scripts
            .get(index)
            .map(String::as_str)
            .unwrap_or_default();
        let location = Some("HTML markup".to_owned());
        if let Some(src) = &slot.src
            && source.is_empty()
        {
            let mut finding = Finding::new(
                Area::ScriptExecution,
                format!("{} failed to load", slot.label),
                format!("The script at {src} could not be fetched, so it never runs."),
                Evidence::Observed,
            );
            finding.location = location.clone();
            report.findings.push(finding);
        }
        if source.trim().is_empty() || slot.is_classic() {
            continue;
        }
        let kind = slot.kind.as_deref().unwrap_or_default();
        let (feature, detail) = if kind == "module" {
            (
                "<script type=\"module\">".to_owned(),
                "Module scripts run as classic scripts: imports, exports, and deferred module \
                 timing are not supported."
                    .to_owned(),
            )
        } else {
            (
                format!("<script type=\"{kind}\">"),
                "The browser runs every <script> as JavaScript regardless of its type, so this \
                 data block is executed and usually fails, stopping later scripts."
                    .to_owned(),
            )
        };
        let mut finding = Finding::new(Area::ScriptExecution, feature, detail, Evidence::Observed);
        finding.location = location;
        report.findings.push(finding);
    }
}

fn audit_syntax<E: PageScriptEngine>(
    auditor: &Auditor<E>,
    scripts: &[ScriptTokens],
    report: &mut AuditReport,
) {
    for feature in SYNTAX {
        let mut usage = Usage::default();
        for script in scripts {
            for i in 0..script.tokens.len() {
                if (feature.detect)(&script.tokens, i) {
                    usage.add(feature.name, || script.location(i));
                }
            }
        }
        let Some((_, used)) = usage.iter().next() else {
            continue;
        };
        report.features_checked += 1;
        let expect = feature.expect;
        if !auditor.script_works(feature.probe, |value| expect.matches(value)) {
            report.findings.push(
                Finding::new(
                    Area::JavaScriptSyntax,
                    feature.name,
                    format!(
                        "Not supported by the JavaScript engine (probe: `{}`).",
                        feature.probe
                    ),
                    Evidence::Probed,
                )
                .at(used),
            );
        }
    }
}

/// One API reference found in the page.
#[derive(Default)]
struct ApiUse {
    count: usize,
    location: Option<String>,
}

fn audit_apis<E: PageScriptEngine>(
    auditor: &Auditor<E>,
    scripts: &[ScriptTokens],
    report: &mut AuditReport,
) {
    // Names the page defines itself, and `Global.member` it assigns.
    let mut declared = HashSet::new();
    let mut assigned = HashSet::new();
    for script in scripts {
        let t = &script.tokens;
        for i in 0..t.len() {
            if ["var", "let", "const", "function", "class"]
                .iter()
                .any(|k| keyword(t, i, k))
                && let Some(name) = t.get(i + 1).and_then(Token::ident)
            {
                declared.insert(name.to_owned());
            }
            if i + 3 < t.len()
                && t[i + 1].is_punct(".")
                && t[i + 3].is_punct("=")
                && let (Some(root), Some(member)) = (t[i].ident(), t[i + 2].ident())
            {
                assigned.insert(format!("{root}.{member}"));
                if root == "window" || root == "self" || root == "globalThis" {
                    declared.insert(member.to_owned());
                }
            }
        }
    }

    let mut globals: BTreeMap<String, ApiUse> = BTreeMap::new();
    let mut members: BTreeMap<(String, String), ApiUse> = BTreeMap::new();
    // (interface key, member) -> use; the receiver is looked up again later.
    let mut instances: BTreeMap<(String, String), ApiUse> = BTreeMap::new();
    let add = |entry: &mut ApiUse, script: &ScriptTokens, i: usize| {
        entry.count += 1;
        if entry.location.is_none() {
            entry.location = Some(script.location(i));
        }
    };
    for script in scripts {
        let t = &script.tokens;
        for i in 0..t.len() {
            let Some(name) = t[i].ident() else {
                continue;
            };
            let member = is_member_name(t, i);
            if !member {
                let object_key = t.get(i + 1).is_some_and(|n| n.is_punct(":"))
                    && prev(t, i).is_some_and(|p| p.is_punct("{") || p.is_punct(","));
                if GLOBALS.contains(&name) && !declared.contains(name) && !object_key {
                    add(globals.entry(name.to_owned()).or_default(), script, i);
                    if NAMESPACES.contains(&name)
                        && t.get(i + 1)
                            .is_some_and(|n| n.is_punct(".") || n.is_punct("?."))
                        && let Some(property) = t.get(i + 2).and_then(Token::ident)
                    {
                        let window_like = matches!(name, "window" | "self");
                        let checked = !window_like || GLOBALS.contains(&property);
                        if checked && !assigned.contains(&format!("{name}.{property}")) {
                            add(
                                members
                                    .entry((name.to_owned(), property.to_owned()))
                                    .or_default(),
                                script,
                                i,
                            );
                        }
                    }
                }
                continue;
            }
            // `name` follows `.`: skip members of namespaces (handled above).
            let parent = i.checked_sub(2).and_then(|j| t[j].ident());
            if parent.is_some_and(|parent| {
                NAMESPACES.contains(&parent)
                    && i >= 3
                    && !t[i - 3].is_punct(".")
                    && !declared.contains(parent)
            }) {
                continue;
            }
            if let Some(parent) = parent
                && let Some(receiver) = path_receiver(parent, name)
            {
                add(
                    instances
                        .entry((receiver.interface.to_owned(), name.to_owned()))
                        .or_default(),
                    script,
                    i,
                );
                continue;
            }
            for receiver in instance_receivers(name) {
                add(
                    instances
                        .entry((receiver.interface.to_owned(), name.to_owned()))
                        .or_default(),
                    script,
                    i,
                );
            }
        }
    }

    let mut missing_globals = HashSet::new();
    for (name, used) in &globals {
        report.features_checked += 1;
        if auditor.exists("", name) {
            continue;
        }
        missing_globals.insert(name.clone());
        report.findings.push(api_finding(
            name,
            format!("`{name}` is not defined in the page's global scope."),
            Evidence::Probed,
            used,
        ));
    }
    for ((root, property), used) in &members {
        if missing_globals.contains(root) {
            continue;
        }
        report.features_checked += 1;
        if auditor.exists("", &format!("{root}.{property}")) {
            continue;
        }
        report.findings.push(api_finding(
            &format!("{root}.{property}"),
            format!("`{root}` exists but has no `{property}`."),
            Evidence::Probed,
            used,
        ));
    }
    // Members of a value the probe cannot even create are reported once
    // per interface rather than once per member.
    let mut uncreatable: BTreeMap<&str, (Vec<&str>, ApiUse)> = BTreeMap::new();
    for ((interface, name), used) in &instances {
        let Some(receiver) = all_receivers(name)
            .into_iter()
            .find(|receiver| receiver.interface == interface)
        else {
            continue;
        };
        if missing_globals.contains(receiver.interface) {
            continue;
        }
        report.features_checked += 1;
        if !auditor.exists(receiver.setup, receiver.expression) {
            // A sub-object such as `classList` is already reported as a
            // missing member of its parent.
            if !receiver.interface.contains('(') {
                let (members, total) = uncreatable.entry(receiver.interface).or_default();
                members.push(name);
                total.count += used.count;
                if total.location.is_none() {
                    total.location.clone_from(&used.location);
                }
            }
            continue;
        }
        if auditor.exists(receiver.setup, &format!("{}.{name}", receiver.expression)) {
            continue;
        }
        let evidence = if receiver.interface.contains('(') {
            Evidence::Probed
        } else {
            Evidence::NameMatch
        };
        report.findings.push(api_finding(
            &format!(
                "{}.{name}",
                receiver.interface.split(' ').next().unwrap_or_default()
            ),
            format!(
                "{} values have no `{name}`.{}",
                receiver.interface,
                if evidence == Evidence::NameMatch {
                    format!(
                        " Matched by member name, so the page may call `{name}` on something else."
                    )
                } else {
                    String::new()
                }
            ),
            evidence,
            used,
        ));
    }
    for (interface, (members, used)) in uncreatable {
        let receiver = all_receivers(members[0])
            .into_iter()
            .find(|receiver| receiver.interface == interface)
            .map_or("", |receiver| receiver.expression);
        let names: Vec<String> = members.iter().map(|name| format!("`{name}`")).collect();
        report.findings.push(api_finding(
            &format!("{interface} values"),
            format!(
                "The engine cannot create a {interface} (`{receiver}` fails), so the page's uses \
                 of {} cannot work. Matched by member name.",
                names.join(", ")
            ),
            Evidence::NameMatch,
            &used,
        ));
    }
}

/// Every receiver that could own member `name`, including sub-object paths.
fn all_receivers(name: &str) -> Vec<Receiver> {
    let mut receivers: Vec<Receiver> = instance_receivers(name).to_vec();
    receivers.extend(path_receiver("classList", name));
    receivers.extend(path_receiver("style", name));
    receivers
}

fn api_finding(feature: &str, detail: String, evidence: Evidence, used: &ApiUse) -> Finding {
    let mut finding = Finding::new(Area::JavaScriptApi, feature, detail, evidence);
    finding.occurrences = used.count.max(1);
    finding.location = used.location.clone();
    finding
}
