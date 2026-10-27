//! Instructions understood by the bytecode virtual machine.

use crate::ast::{BinaryOperator, UnaryOperator, VarKind};

/// A single stack-machine instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instruction {
    /// Pushes a constant from the chunk's constant pool.
    Constant(u16),
    /// Discards the value at the top of the stack.
    Pop,
    /// Swaps the two values at the top of the stack.
    Swap,
    /// Jumps unconditionally to an instruction offset.
    Jump(usize),
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
    /// A missing property reads as `undefined`. Reading any property of a
    /// non-object is a runtime error, matching the tree-walk evaluator.
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
    /// Pushes a new empty object, the start of an object literal.
    NewObject,
    /// Adds an object literal property: pops the value, stores it on the
    /// object now at the top of the stack under the string constant at this
    /// pool index, and leaves the object on the stack.
    DefineProperty(u16),
    /// Adds a computed object literal property: pops the value, then the
    /// key, stores the value under the stringified key on the object now at
    /// the top of the stack, and leaves the object on the stack.
    DefineComputedProperty,
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
