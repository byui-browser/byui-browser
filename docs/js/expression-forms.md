# JavaScript expression forms

This page lists the expression syntax the engine supports beyond plain
arithmetic, comparison, calls, and member access, how each form evaluates, and
what is rejected. Every form below parses, compiles to bytecode, and runs in
`Vm` with no evaluator fallback, and the tree-walk interpreter implements the
same semantics. `crates/js/tests/expressions.rs` runs each form through both
engines and requires the same value, error, and side effects.

## Supported forms

| Form | Notes |
|---|---|
| `test ? a : b` | Right associative. Only the chosen branch runs. |
| `a, b, c` | Evaluates every operand and yields the last. Commas in call arguments, array literals, object literals, and declarations still separate items. |
| `++x`, `x++`, `--x`, `x--` | Target is a variable or member. The old value is converted with `ToNumber`; a postfix update yields that number. `++` must be on the same line as its operand. |
| `x += y` and 11 more | `+= -= *= /= %= **= <<= >>= >>>= &= \|= ^=`. A member target's object and key are evaluated once, before the right side. |
| `x &&= y`, `x \|\|= y`, `x ??= y` | Skip the right side **and the store** when the left value decides the result, so `const c = 1; c \|\|= 2` is not an error. |
| `a ?? b` | Right side runs only for `null` or `undefined`. Mixing `??` with `\|\|` or `&&` without parentheses is a syntax error, as in JavaScript. |
| `typeof x`, `void x`, `+x` | `typeof` of an undeclared name is `"undefined"` instead of an error; other failures, such as reading an uninitialized `let`, still throw. Arrays and `null` are `"object"`. |
| `delete o.k`, `delete o[k]` | Returns a boolean. Removes own properties, leaving a hole in arrays. `length`, read-only, and host accessor properties are not deleted (`false`). Deleting from `null`/`undefined` or from a function is a runtime error. `delete name` and `delete a?.b` are syntax errors. |
| `k in o`, `v instanceof C` | `in` follows prototype chains for objects and arrays and needs an object, array, or function on the right. `instanceof` needs a function that has an object `prototype`, which only bytecode constructors (functions and classes) have. A non-object left side is `false`. |
| `a ** b` | Right associative; `-a ** b` is a syntax error. `1 ** Infinity` and a `NaN` exponent give `NaN`, as in JavaScript. |
| `` `a${b}c` `` | Substitutions are full comma expressions and can nest templates. Text is joined as `"" + value` would join it. Line breaks are kept (`\r\n` becomes `\n`). |
| `[...a]`, `f(...a)`, `new C(...a)`, `{ ...o }` | Array spread accepts arrays and strings; holes read as `undefined`. Object spread copies own enumerable properties of objects, arrays (indices, not `length`), and strings, and ignores `null`, `undefined`, and other primitives. |
| `a?.b`, `a?.[k]`, `a?.(x)` | A `null` or `undefined` base ends the **whole chain** (`a?.b.c.d`) with `undefined` and skips the remaining operands. A parenthesized chain ends the short circuit. |

Evaluation order is left to right everywhere, including the object-then-key
order of member targets. Reference errors for undeclared names, assignment to
a constant, and use before initialization come from the same instruction in
both engines.

## Rejected syntax

Each of these is rejected deterministically while parsing (a `Syntax` error
with a line and column) and never reaches the evaluator:

- Regular-expression literals (`/ab+c/g`): the engine has no regular-expression
  value, so `/` and `/=` where an operand must start are errors.
- Tagged templates (`tag\`x\``).
- `delete name` (bare identifier) and `delete` of an optional chain.
- `-a ** b` and the other unary operators directly before `**`.
- Mixing `??` with `||`/`&&` without parentheses.
- Assigning to, updating, or `new`-ing an optional chain (`a?.b = 1`,
  `a?.b++`, `new a?.b()`).
- Update or compound assignment on anything but a variable or member
  (`1++`, `(a + b) += 1`).
- Rest parameters and rest in literals (`function f(...a)`), and spread
  anywhere but an array literal, call/`new` arguments, or an object literal.
- Function expressions, `async`/`await`, generators, destructuring, getters
  and setters, `label:` statements, and other syntax not listed here remain
  unsupported as before.

## Known limitations

These parse and run, but differ from full JavaScript. Each is documented next
to its implementation:

- **Method calls in the tree-walk interpreter do not receive `this`**; the VM
  does. Differential tests avoid `this`.
- **`in` and array holes**: an elision such as `[1, , 3]` stores `undefined`,
  so `1 in [1, , 3]` is `true`. A hole made by `delete a[1]` is truly absent.
- **`instanceof` on the interpreter**: script functions have no `prototype`,
  so it throws "Function has a non-object prototype". The VM supports classes
  and constructor functions.
- **Template literals and `+`**: substitutions use the engine's `+`
  conversion (`valueOf` before `toString` for objects), not `ToString`.
- **No iterator protocol**: spread needs an array or a string.
- **`delete` on functions** is a runtime error instead of a property removal.
- **Large array literals**: a literal with 65,536 or more elements without
  spread is not compiled; build it with spread or a loop.
