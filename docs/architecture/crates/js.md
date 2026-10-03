# Crate: `js`

**Purpose:** the JavaScript engine: lexer, parser, AST, interpreter, values,
and the generic host-function interface that `webapis` builds on. Knows
nothing about the browser.
**Maintained by:** JavaScript Engine Team.
**Contract:** [`../contracts/scripting.md`](../contracts/scripting.md).

## Dependencies

- May depend on: `common`.
- Depended on by: `webapis`, `devtools`, `browser`.
- Must never depend on: `webapis`, `html`, `net`, or any crate with a
  Web-platform name in it.

## Intended public contract

See `contracts/scripting.md` §`js` public contract. In short:

- `Value` (grows to `Object`/`Function`), `JsError`, `JsResult`.
- `Realm` as the **interpreter's global environment**:
  `register_global_function`, `register_global_value`, `evaluate_script`
  (real parse + run), `call_function` (for callbacks from the event loop).
- `HostFunction = Box<dyn FnMut(&mut HostContext, &[Value]) -> JsResult<Value>>`,
  not `Send + Sync`.
- `parse(&str) -> Result<Program, ParseError>` and the AST stay public for
  DevTools and tests.
- Execution strategy (tree-walking vs bytecode) and memory strategy
  (`Rc`/arena/GC) are internal and may change without touching the contract.

Language milestones, in the order that unblocks `webapis`: function calls
(host and script), `if`/`while`/`return`, string `+` and proper `==`/`===`,
objects and property access (`a.b`, `a["b"]`), closures, arrays, then the
standard library.

## Current state (2026-10-03)

About 1,900 lines; the second-largest crate.

- **Lexer** (`lexer/`): numbers, strings with escapes, identifiers, the
  keywords `let var const if else while function return true false null
  undefined`, operators `+ - * / = == === ! != !== < <= > >= && ||`, and
  `; ( ) { } ,`. No comments, no `.`/`[]`, no `%`, no Unicode identifiers.
  Never fails (emits `Unknown`/`Invalid` tokens).
- **Parser** (`parser/`): recursive descent with precedence climbing,
  128-level nesting limit, no automatic semicolon insertion, rich
  `ParseError` with line/column. Good test coverage.
- **AST** (`ast/`): fully documented.
- **Interpreter** (`runtime/`): tree-walking over a `Vec<HashMap>` scope
  stack. Supports expression statements, `let`/`var`/`const` (no hoisting),
  blocks. `if`, `while`, `function`, `return`, and **all calls** return
  "not supported" errors. Every binary operator coerces to number, so `+`
  never concatenates and `'a' == 'a'` is an error. `runtime::evaluate`
  swallows errors as `Undefined`.
- **`Realm`** (`lib.rs`): a `HashMap<String, HostFunction>` with
  `register_global_function`, `call_global`, and `evaluate_script`, which
  **does not use the parser or interpreter**: it trims the source, strips
  `()`, and looks the name up. Arguments cannot be passed from script.
  `Realm` and `runtime::Environment` do not know about each other. (G-09)
- `HostFunction = Arc<dyn Fn(&[Value]) -> JsResult<Value> + Send + Sync>`;
  no `HostContext` (G-31).
- `src/main.rs` is a demo binary with nine `println!`s (G-26).
- Crate doc claims "bytecode compiler, interpreter/VM, GC" (G-28).
- `common` declared but unused (G-22).

## Gaps owned by this crate

G-09 is the critical path for the whole scripting stack: make `Realm` own an
`Environment`, route `evaluate_script` through the parser and interpreter,
and make `Expr::Call` resolve globals to host functions. Then G-31
(`HostContext`, drop `Send + Sync`), string semantics, control flow. G-26,
G-28 are quick cleanups.

## Tests

- `tests/lexer.rs` (8), `tests/parser.rs` (11), `tests/contract.rs` (12,
  interpreter behaviour). Strong for what exists.
- **No test covers `Realm`** directly; it is exercised only from `webapis`.
- No fixtures; add `tests/js/` when programs get longer than a line.

## Read next

`contracts/scripting.md`, then `crates/webapis.md` to see what the Web APIs
team is waiting on.
