//! Bytecode representation and compiler for the JavaScript engine.
//!
//! This module is the first compiler boundary between the parser's [`crate::ast`]
//! and the stack-based [`crate::vm`]. The instruction set covers expressions,
//! lexical bindings, object literals, property access (`object.name`,
//! `object[key]`, and assignment to either), conditional branches, `while`
//! loops, top-level function declarations, calls, `return`, and classes
//! (`class`, `extends`, `new`, `this`, `super`).
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
