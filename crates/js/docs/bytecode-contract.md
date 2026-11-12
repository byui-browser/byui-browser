# Bytecode contract

The VM consumes a [`Chunk`](../src/bytecode/chunk.rs) produced by
`bytecode::compile`. A chunk is immutable input to execution; its instruction
offsets address `code`, constants address `constants`, binding names address
`names`, and function operands address `functions`. `locations[i]` is optional
source metadata for `code[i]`; a function template owns a separate chunk and
therefore has its own offset space. Disassembly is deterministic and prints
instruction offsets, operands, constants, and recorded line/column locations.
The parser/compiler currently do not attach source spans, so compiler-produced
locations are absent; callers constructing chunks may attach them with
`emit_at`.

## Stack effects

Effects below are relative to the operand stack. `v` means a value, `n` an
operand count, and `-` means no net change. Branches and errors may terminate
the current path. A runtime error is catchable only when an active bytecode
handler covers the instruction.

| Instruction group | Stack effect | Contract |
| --- | --- | --- |
| `Constant`, `Load*`, `LoadThis`, `LoadArgument`, `MakeFunction`, `NewObject` | `+1` | Push a value; loads may fail for invalid/uninitialized bindings. |
| `Pop` | `-1` | Discard the top value. |
| `Dup` | `+1` | Duplicate the top value. |
| `Swap` | `0` | Exchange the top two values. |
| `Unary`, `LoadProperty`, `LoadSuperProperty` | `0` | Replace one value with one value. |
| `Binary`, `LoadComputedProperty` | `-1` | Consume two values and push one. |
| `StoreProperty` | `-1` | Consume object and value; push assigned value. |
| `StoreComputedProperty` | `-2` | Consume object, key, and value; push assigned value. |
| `DefineProperty` | `-1` | Consume property value; retain object. |
| `DefineComputedProperty` | `-2` | Consume key and value; retain object. |
| `NewArray(n)` | `1-n` | Consume `n` ordered values and push the array. |
| `Call(n)`, `CallMethod(n)`, `New(n)`, `SuperCall(n)` | `-n` | Consume callee/receiver and `n` arguments; push result. |
| `DefineClass` | `+1` without superclass; `0` with one | Consume optional superclass and push class. |
| `DefineMethod` | `0` | Attach method to class on top; retain class. |
| `Jump`, `EnterTry`, `ExitTry`, `EnterScope`, `ExitScope` | `0` | Control/environment state changes only. |
| `JumpIfFalse`, `JumpIfTrue` | `0` | Branch based on top value without consuming it. |
| `Throw` | `-1` | Consume thrown value; transfer to handler or return an error. |
| `Return` | terminal | Return top value, or `undefined` for an empty stack. |
| `Declare*`, `Initialize*` | `0` / `-1` | Declaration does not touch stack; initialization consumes its value. |
| `StoreGlobal`, `StoreLocal` | `0` | Replace the assigned value in place, leaving it as expression result. |
| `LoadUnresolved` | `+1` on success | Always reports an unresolved-name error. |
| `StoreUnresolved` | `0` on success | Always reports an unresolved-name error. |

The VM validates operands as they execute. Invalid constant, name, function,
slot, jump, stack, scope, or handler references produce a runtime error rather
than being trusted or causing an unchecked index access. Hand-built chunks
must keep parallel `code` and `locations` entries aligned; missing location
metadata is treated as absent.

## Function boundaries

`MakeFunction` creates a callable from a function-pool template. Each call has
its own frame, operand base, arguments, receiver, lexical scope chain, and
exception handlers. Parameters missing from the call are `undefined`; extra
arguments are ignored. Ordinary functions are constructible, arrows and class
methods are not, and class constructors must be invoked with `New`. A method
call passes its receiver as `this`; an ordinary call uses `undefined`.
Top-level bindings are visible in functions. Capturing enclosing function or
block locals and lexical `this` in arrows are not supported by this VM yet.

## Tests and evolution

Sprint 4 conformance tests compile then execute through `Vm` directly. They
assert outcomes and errors without evaluator fallback. When adding an AST
variant, add VM coverage or explicitly report its compile rejection; do not
silently route a VM test to the tree-walk evaluator. Changes to an instruction's
stack effect, control transfer, call boundary, or metadata interpretation are
bytecode contract changes and require updating this document and disassembly
or malformed-bytecode regression tests.
