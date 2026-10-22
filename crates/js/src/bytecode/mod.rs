//! Bytecode representation and compiler for the JavaScript engine.
//!
//! This module is the first compiler boundary between the parser's [`crate::ast`]
//! and the stack-based [`crate::vm`]. The instruction set currently covers
//! expressions, lexical bindings, and property access (`object.name`,
//! `object[key]`, and assignment to either). Calls, functions, and control
//! flow are reported as compile errors until their corresponding instructions
//! are added.

mod chunk;
mod compiler;
mod instruction;

pub use chunk::Chunk;
pub use compiler::{CompileError, Compiler};
pub use instruction::Instruction;

/// Compiles a parsed program into an executable bytecode chunk.
pub fn compile(program: &crate::Program) -> Result<Chunk, CompileError> {
    Compiler::new().compile(program)
}
