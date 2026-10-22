//! Instructions understood by the bytecode virtual machine.

use crate::ast::{BinaryOperator, UnaryOperator, VarKind};

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
    /// Enters a lexical environment with `slot_count` slots.
    EnterScope(u16),
    /// Leaves the current lexical environment.
    ExitScope,
    /// Declares a global binding before the program body runs.
    DeclareGlobal {
        /// Slot index in the global environment.
        slot: u16,
        /// Binding mutability and declaration kind.
        kind: VarKind,
        /// Index into [`Chunk::names`](super::Chunk::names).
        name: u16,
    },
    /// Declares a binding in the current lexical environment.
    DeclareLocal {
        /// Slot index in the current lexical environment.
        slot: u16,
        /// Binding mutability and declaration kind.
        kind: VarKind,
        /// Index into [`Chunk::names`](super::Chunk::names).
        name: u16,
    },
    /// Reads a binding from the global environment.
    LoadGlobal {
        /// Slot index in the global environment.
        slot: u16,
        /// Index into [`Chunk::names`](super::Chunk::names).
        name: u16,
    },
    /// Reads a binding from an enclosing lexical environment.
    LoadLocal {
        /// Number of lexical environments between the current and target scope.
        depth: u16,
        /// Slot index in the target environment.
        slot: u16,
        /// Index into [`Chunk::names`](super::Chunk::names).
        name: u16,
    },
    /// Initializes a global binding, leaving its value on the stack.
    InitializeGlobal {
        /// Slot index in the global environment.
        slot: u16,
        /// Index into [`Chunk::names`](super::Chunk::names).
        name: u16,
    },
    /// Initializes a local binding, leaving its value on the stack.
    InitializeLocal {
        /// Slot index in the current lexical environment.
        slot: u16,
        /// Index into [`Chunk::names`](super::Chunk::names).
        name: u16,
    },
    /// Assigns a global binding, leaving the assigned value on the stack.
    StoreGlobal {
        /// Slot index in the global environment.
        slot: u16,
        /// Index into [`Chunk::names`](super::Chunk::names).
        name: u16,
    },
    /// Assigns a local binding, leaving the assigned value on the stack.
    StoreLocal {
        /// Number of lexical environments between the current and target scope.
        depth: u16,
        /// Slot index in the target environment.
        slot: u16,
        /// Index into [`Chunk::names`](super::Chunk::names).
        name: u16,
    },
    /// Reports a read of a name that is not declared in the compiled program.
    LoadUnresolved {
        /// Index into the chunk binding-name pool.
        name: u16,
    },
    /// Reports an assignment to a name that is not declared in the compiled program.
    StoreUnresolved {
        /// Index into the chunk binding-name pool.
        name: u16,
    },
}
