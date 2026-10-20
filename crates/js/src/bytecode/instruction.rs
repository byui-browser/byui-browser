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
    /// Reads a named property: pops the object and pushes the value stored
    /// under the string constant at this pool index.
    ///
    /// Reading a missing property, or any property of a non-object, is a
    /// runtime error, matching the tree-walk evaluator.
    LoadProperty(u16),
    /// Reads a computed property: pops the key, then the object, and pushes
    /// the value. The key is converted to a string, so `1` and `'1'` name the
    /// same property.
    LoadComputedProperty,
    /// Writes a named property: pops the value, then the object, stores the
    /// value under the string constant at this pool index, and pushes the
    /// value back as the expression's result.
    ///
    /// The target is checked only after the value has been evaluated, so a
    /// non-object target fails after the value's side effects.
    StoreProperty(u16),
    /// Writes a computed property: pops the value, the key, then the object,
    /// stores the value under the stringified key, and pushes the value back.
    StoreComputedProperty,
}
