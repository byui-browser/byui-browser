# Execution mode migration

`eval_with_mode` provides a controlled bytecode boundary. `Automatic` compiles
once per script and runs a successful chunk in the VM. It falls back to the
tree-walk interpreter only when the compiler returns its explicit unsupported
syntax result. Compilation diagnostics and VM runtime or limit errors are
returned; they never trigger a second evaluation.

Compiled chunks support top-level function declarations, ordered argument
evaluation, parameters, returns, recursion, and class methods. Functions that
capture enclosing local bindings and nested or block function declarations are
currently unsupported by the compiler. `Automatic` falls back to the
interpreter for those constructs. Each VM call uses a frame with its own chunk,
instruction pointer, operand-stack base, and function scope. Call depth is
bounded by `ExecutionLimits::call_depth_limit`.

Exceptions (`throw`, `try`/`catch`/`finally`) and loop control (`break`,
`continue` in `while`) compile to handler instructions and are resolved
entirely inside the VM; see the `bytecode` module documentation for the
design. The tree-walk interpreter implements the same semantics so
`Realm` scripts and the `Automatic` fallback behave identically, and
`tests/exceptions.rs` compares the two engines. Limit errors are never
catchable and skip `finally` blocks in both engines.

`Interpreter` remains available for differential comparisons. `Bytecode`
requires the VM path. Current VM chunks own fresh bindings, so `Realm` keeps
using the interpreter until persistent global slots and host-value resolution
are implemented in VM frames. Realm scripts therefore preserve their existing
global state and host side effects during this migration.

Track migration by extending `crates/js/tests/differential.rs` with programs
that compile successfully and asserting interpreter/VM completion values,
error categories, and shared-object effects. Keep unsupported constructs in a
separate fallback-boundary list and move each to the VM corpus as compiler
support lands. The current bytecode subset and VM execution tests are in
`crates/js/tests/bytecode.rs`.
