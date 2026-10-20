//! Instructions understood by the bytecode virtual machine.

use crate::ast::{BinaryOperator, UnaryOperator};

/// A single stack-machine instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instruction {
    /// Pushes a constant from the chunk's constant pool.
    Constant(u16),
    /// Discards the value at the top of the stack.
    Pop,
    /// Returns the top value, or `undefined` when the stack is empty.
    Return,
    /// Applies a unary operator to the top stack value.
    Unary(UnaryOperator),
    /// Applies a binary operator to the two top stack values.
    Binary(BinaryOperator),
    /// Jumps when the top value is falsy without consuming it.
    JumpIfFalse(usize),
    /// Jumps when the top value is truthy without consuming it.
    JumpIfTrue(usize),
}
