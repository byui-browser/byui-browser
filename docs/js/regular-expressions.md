# Regular expressions

The source parser recognizes regular-expression literals where an operand is
expected, and treats `/` and `/=` after an operand as arithmetic operators.
It preserves escapes and character classes while scanning the literal, then
resumes ordinary JavaScript tokenization after the closing slash and flags.
This also prevents quotes and comment markers inside a pattern from consuming
subsequent JavaScript. Token-level parser users must supply `Token::RegExp`;
`lexer::tokenize` alone does not select the regular-expression lexical goal.

Both execution engines create a fresh regular-expression object on every
literal evaluation. The VM's `NewRegExp` instruction stores pattern and flag
constant indices, rather than sharing a mutable object in the constant pool.
Objects hold immutable compiled pattern data in an internal slot; scripts
cannot forge the slot by copying public properties. Each object owns its
writable `lastIndex`, measured in UTF-16 code units.

## Supported API

- `/pattern/flags`, `RegExp(pattern, flags)`, and `new RegExp(pattern, flags)`.
  Calling `RegExp` with a regex and no flags returns that object; construction
  clones it and starts at `lastIndex = 0`.
- Flags `d`, `g`, `i`, `m`, `s`, `u`, `y`, with canonical ordering in `.flags`.
  Duplicate and unknown flags are errors. The `u` flag has the limitations below.
- `exec(string)` returns `null` or an array with captures, `index`, `input`,
  `groups`, and (with `d`) `indices` including named-group ranges. Unmatched
  captures are `undefined`. `test(string)` returns a boolean.
- `source`, `flags`, `global`, `ignoreCase`, `multiline`, `dotAll`, `unicode`,
  `sticky`, `hasIndices`, and `toString()`.
- Global and sticky execution reads and updates `lastIndex`; failure resets
  it to zero. Ordinary execution ignores it. Empty `exec` matches do not
  advance it, as in JavaScript.
- String `match` and `search`; regex arguments to `replace` and `split`.
  Global matching and replacement advance past empty matches. Search and split
  preserve the supplied regex's `lastIndex`.
- Replacement strings support `$$`, `$&`, prefix/suffix substitutions,
  `$1`–`$99`, and `$<name>`. Replacement functions receive the whole match,
  captures, UTF-16 offset, input, and named groups when present.
- Split includes captured delimiters, accepts a limit, and handles empty
  patterns without adding artificial leading/trailing empty elements.

Patterns support the backend's common grouping, alternation, quantifiers,
character classes, captures, named groups, backreferences, and lookarounds.
The adapter explicitly translates JavaScript's ASCII digit/word classes,
whitespace, word boundaries, control escapes, dot, and line anchors. Backend
extensions such as atomic groups, inline flag groups, and possessive
quantifiers are rejected.

Invalid literals fail during parsing with source locations. Invalid dynamic
patterns/flags throw catchable `SyntaxError` instances; incompatible method
receivers throw `TypeError` instances.

## Compatibility limits

This is a useful initial implementation, not full ECMAScript RegExp conformance.
The backend is `fancy-regex` 0.16 rather than a JavaScript regex VM.

- Matching operates on Rust Unicode scalar values in both `u` and non-`u`
  modes. UTF-16 result indices are reported, but matching individual surrogate
  halves, lone-surrogate escapes, and starting in the middle of a surrogate
  pair are unsupported. The `u` flag does not implement the full Unicode
  escape/validation grammar. Case folding follows the backend's Unicode rules.
- Legacy octal/identity escape corner cases, forward references, unmatched
  backreference behavior, repeated-capture clearing, and variable-length
  lookbehind can differ from JavaScript or be rejected by the backend.
- `v` Unicode sets, `matchAll`, `replaceAll`, legacy RegExp static captures,
  `compile`, species/subclassing, and symbol protocol overrides are unsupported.
  String methods use the internal matcher; overriding `exec` does not alter it.
- Intrinsic regex properties are read-only own properties. Full ECMAScript
  accessor descriptors, enumerability, and generic prototype accessors are
  not modeled by the object system.
- Template substitutions use a lightweight lexical goal to find their closing
  brace before the parser re-parses their source. Regex operands and common
  statement bodies work; arbitrary class/declaration grammar inside a
  substitution is not a complete ECMAScript lexical-goal implementation.
- Patterns are limited to 64 KiB and delegated compiled patterns to 2 MiB.
  The backtracking backend stops after 100,000 backtracking steps with a
  structured `Limit` error. This budget is independent of the JS statement
  budget; linear matching and replacement callbacks use their existing host
  execution paths.

`crates/js/tests/regexp.rs` covers both engines, parsing/division boundaries,
state, replacement, splitting, UTF-16 offsets, and bounded backtracking.
`crates/browser/tests/page_scripts.rs` verifies regex-driven DOM updates across
page scripts without accessing an external sign-in service.
