# JavaScript regression coverage

The JavaScript vertical slice takes JavaScript source text, parses it, runs it
with the tree-walk interpreter, and calls host functions such as
`webapis::print`. Its regression suite uses only the public source-evaluation
APIs: `js::eval`, `js::parse`, and `js::Realm::evaluate_script`.

| File | Covers |
|---|---|
| `crates/js/tests/regression.rs` | Arithmetic precedence, literals, truthiness, comparisons, short-circuiting, declarations, assignment, control flow, functions, returns, recursion, syntax errors, runtime errors, host-function calls, and unsupported syntax |
| `crates/js/tests/contract.rs` | Source-level contract for string literals and structured syntax errors. These tests were previously `#[ignore]`d. |
| `crates/webapis/tests/print_binding.rs` | End to end: JavaScript `print()` reaches `webapis::print` through a `Realm` global and writes to an injected `ConsoleSink` |

Run with:

```sh
cargo test -p js -p webapis
```

The tests are deterministic: they use no timers, no network, and no
randomness. Host functions record calls in memory.

## Intentionally unsupported behavior

The suite checks that each item below fails in a predictable way. When one of
these features is implemented, update or remove the matching case in
`unsupported_syntax_is_rejected_at_parse_time` or
`unsupported_keywords_and_globals_fail_at_run_time`.

### Rejected while parsing (`JsErrorCategory::Syntax`)

- Member access (`o.x`), object literals (`{}` in expressions), and array
  literals.
- `++`/`--` and compound assignment (`+=` and similar).
- Unary `+` and `typeof`.
- Function expressions and arrow functions. Only function declarations are
  supported.
- Template literals.
- `for` loops.
- Declaring several variables in one statement (`let a = 1, b = 2;`).
- Empty statements (`;`, `1;;`).
- `return` outside a function.
- Automatic semicolon insertion. A newline does not end a statement, so
  `let a = 1\nlet b = 2` is an error.
- Using `undefined` as a binding name, because it is a keyword in this engine.
- Hexadecimal, octal, and binary numeric literals in source code. Strings
  containing them, such as `'0x10' * 1`, do still convert to numbers.
- Legacy octal string escapes (`\1`), `\8`, `\9`, and lone surrogate escapes.

### Parsed but fails at run time (`JsErrorCategory::Runtime`)

- `break`, `continue`, and `this` are read as ordinary identifiers, so they
  fail with "`name` is not defined".
- The globals `NaN` and `Infinity` are not defined. Use `0 / 0` and `1 / 0`
  instead.
- Assigning to an undeclared name is an error, as in strict mode.

### Other known limitations

- Errors from host functions are returned unchanged. A host that returns
  `JsError::new` produces a `Runtime` error with no call context. The `Host`
  error category exists, but the interpreter does not use it yet.
- Bindings created by one `Realm::evaluate_script` call are not visible to the
  next call.
- `webapis::print` takes no arguments and always logs a fixed message. Calling
  it with arguments is a runtime error.
