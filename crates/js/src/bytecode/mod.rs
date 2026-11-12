//! Bytecode representation and compiler for the JavaScript engine.
//!
//! This module is the first compiler boundary between the parser's [`crate::ast`]
//! and the stack-based [`crate::vm`]. The instruction set covers expressions,
//! lexical bindings, object literals, property access (`object.name`,
//! `object[key]`, and assignment to either), conditional branches, `while`
//! loops, top-level function declarations, calls, `return`, and classes
//! (`class`, `extends`, `new`, `this`, `super`).
//!
//! # Expression forms
//!
//! Conditional, comma, update, compound and logical assignment, `??`,
//! `typeof`/`void`/`delete`, `in`/`instanceof`, `**`, template literals,
//! spread, and optional chaining all compile to bytecode; none falls back to
//! the tree-walk evaluator. The forms, their evaluation order, and what is
//! rejected are listed in `docs/js/expression-forms.md`.
//!
//! - Conditionals and `&&`/`||`/`??` branch with `JumpIfFalse`, `JumpIfTrue`,
//!   and `JumpIfNullish`, which peek at the top value, so each branch pops it.
//! - A compound assignment or update to a member evaluates the object (and
//!   key) once. [`Instruction::Dup2`] and [`Instruction::MoveTop`] keep that
//!   reference on the stack beside the value being read.
//! - An optional chain emits one `JumpIfNullish` per `?.`. Each jumps to a
//!   landing pad that pops the checked value and anything the chain kept
//!   beneath it, then pushes `undefined`, so all paths leave one value.
//! - A call, `new`, or array literal with spread builds an array with
//!   `ArrayPush`/`ArrayExtend` and uses `CallSpread`, `CallMethodSpread`, or
//!   `NewSpread`; object spread uses `SpreadProperties`.
//!
//! # Exceptions and abrupt completion
//!
//! `throw`, `try`/`catch`/`finally`, `break`, and `continue` are lowered to
//! three instructions, [`Instruction::Throw`], [`Instruction::EnterTry`], and
//! [`Instruction::ExitTry`], and the VM resolves every exception itself; the
//! host never sees an exception unless nothing in the script handles it.
//!
//! - `EnterTry(target)` registers a handler recording the operand stack
//!   height and lexical scope depth. When a catchable error is raised inside
//!   the region, from the same frame or any callee, the VM discards the
//!   callee frames, restores those two values, pushes the thrown value, and
//!   jumps to `target`. A handler that fires is removed; `ExitTry` removes
//!   one that did not.
//! - A `finally` block is compiled inline at the end of the normal path, once
//!   for exceptions (ending in `Throw`, which rethrows), and again at every
//!   `return`, `break`, or `continue` that leaves its `try`. Each of those
//!   exits first emits `ExitTry` and the `ExitScope`/`Pop` instructions that
//!   restore the scope depth and stack the block expects. Nested `finally`
//!   blocks therefore grow code size multiplicatively; this is the same
//!   trade-off as in other inline-`finally` compilers.
//! - A `finally` block that completes abruptly (`return`, `break`,
//!   `continue`, or a throw) replaces the outcome it interrupted.
//! - Runtime and host errors are exceptions: `catch` receives an object with
//!   string `name` and `message` properties, and rethrowing that object
//!   unchanged restores the original error. Limit errors are not
//!   exceptions: they skip `catch` and `finally` and end the script.
//! - An uncaught `throw` fails with [`JsErrorCategory::Thrown`](crate::JsErrorCategory)
//!   and the thrown value in [`JsError::thrown`](crate::JsError::thrown).
//!
//! Functions and methods see their own parameters and locals plus the
//! top-level bindings. They cannot capture variables of an enclosing block or
//! function: the compiler rejects such a reference instead of miscompiling it.

mod chunk;
mod compiler;
mod instruction;

pub use chunk::{CallableKind, Chunk, FunctionTemplate, SourceLocation};
pub use compiler::{CompileError, Compiler};
pub use instruction::Instruction;

/// Compiles a parsed program into an executable bytecode chunk.
pub fn compile(program: &crate::Program) -> Result<Chunk, CompileError> {
    Compiler::new().compile(program)
}
