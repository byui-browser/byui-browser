# JavaScript value and operator semantics (Sprint 2 subset)

This page documents what the tree-walk interpreter guarantees for the
primitive `Value` types and the supported operators. Bytecode and the byte
compiler are out of scope. The behavior is implemented in
`crates/js/src/runtime/conversions.rs` and `crates/js/src/runtime/operators.rs`
and is pinned by `crates/js/tests/operators.rs`.

## Conversions

| Value | `ToNumber` | `ToBoolean` | `ToString` (used by `+`) |
|---|---|---|---|
| `undefined` | `NaN` | `false` | `"undefined"` |
| `null` | `0` | `false` | `"null"` |
| `true` / `false` | `1` / `0` | itself | `"true"` / `"false"` |
| number | itself | `false` for `0`, `-0`, `NaN`; otherwise `true` | shortest round-trip digits; `NaN`, `Infinity`, `-Infinity`; `-0` prints as `0` |
| string | trimmed; `""` is `0`; decimal, `0x`/`0o`/`0b`, and `±Infinity` are numbers; anything else is `NaN` | `false` only for `""` | itself |

Conversion never fails and never panics. A value that cannot be converted
becomes `NaN`.

## Operators

- **`+`**: if either operand is a string, the other operand is converted to
  text and the two are concatenated (`'n=' + 1` is `"n=1"`, `1 + 2 + '3'` is
  `"33"`). Otherwise both operands are converted to numbers and added.
  Only `+` concatenates; `-`, `*`, `/`, and `%` always convert to numbers.
- **`-`, `*`, `/`, `%`**: operate on converted numbers using IEEE 754 doubles.
  `%` has the same precedence as `*` and `/`, and the result takes the sign of
  the left operand (`-7 % 3` is `-1`).
- **Division by zero** is not an error: `1 / 0` is `Infinity`, `-1 / 0` is
  `-Infinity`, `0 / 0` is `NaN`, and `1 % 0` is `NaN`.
- **`NaN`** is not equal to anything, including itself, under `==`, `===`,
  `!=` and `!==`; every ordering comparison involving it is `false`.
- **`===` / `!==`**: never coerce. Values of different types are unequal.
  Numbers compare by value (`0 === -0`).
- **`==` / `!=`**: `null` and `undefined` equal each other and nothing else;
  number/string and boolean operands are compared as numbers.
- **`<`, `>`, `<=`, `>=`**: two strings compare by UTF-16 code units; otherwise
  both sides are converted to numbers.
- **`&&`, `||`**: return one of their operands, not a boolean, and do not
  evaluate the right side when the left side decides the result.
- **Unary `-`, `!`, `~`**: convert with `ToNumber`, `ToBoolean`, and `ToInt32`
  respectively.
- **Bitwise and shift operators** convert to 32-bit integers.

## Errors

Operators themselves do not fail. Failures come from evaluating an operand
(for example an undefined variable) and are returned as
`JsError` values with category `Runtime`. Unsupported operator syntax
(`typeof`, unary `+`, `**`, `??`, `++`/`--`, compound assignment) is rejected
with a `Syntax` error.

## Explicitly out of scope

- Object, array, and prototype behavior. Objects and arrays convert through a
  simplified `ToPrimitive` (`valueOf`, then `toString`, then
  `"[object Object]"`); an error thrown from those methods is ignored rather
  than propagated. This is a documented limitation, not a supported feature.
- `BigInt`, `Symbol`, and locale-aware conversions.
- Bytecode execution and the byte compiler.
