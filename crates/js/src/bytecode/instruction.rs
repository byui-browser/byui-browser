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
    /// Duplicates the value at the top of the stack.
    Dup,
    /// Pushes the running function's `this` receiver.
    ///
    /// At the top level this is `undefined`. In a derived constructor it is
    /// an error until `super(...)` has returned.
    LoadThis,
    /// Pushes the call argument at this position, or `undefined` when the
    /// caller passed fewer arguments.
    LoadArgument(u16),
    /// Pushes a new function value made from the template at this index of
    /// [`Chunk::functions`](super::Chunk::functions).
    MakeFunction(u16),
    /// Calls a function with `this` set to `undefined`: pops the arguments,
    /// then the callee, and pushes the result. The operand is the argument
    /// count.
    ///
    /// Class constructors cannot be called this way.
    Call(u16),
    /// Calls a method: pops the arguments, then the callee, then the receiver
    /// that becomes `this`, and pushes the result.
    CallMethod(u16),
    /// Constructs an instance: pops the arguments, then the constructor, and
    /// pushes the new object, or the object the constructor returned.
    ///
    /// A constructor that returns a non-object value yields the new instance
    /// instead. The instance's prototype is the constructor's `prototype`
    /// property.
    New(u16),
    /// Runs the parent constructor from a derived constructor: pops the
    /// arguments, binds `this` to the constructed instance, and pushes it.
    ///
    /// Calling it twice in one constructor is an error.
    SuperCall(u16),
    /// Reads a method through `super`: pushes the property named by the
    /// string constant at this pool index, looked up starting from the parent
    /// of the running method's home prototype.
    LoadSuperProperty(u16),
    /// Defines a class: pops the superclass when `has_superclass` is set, and
    /// pushes the class constructor made from the template at `function`.
    ///
    /// The class gets a `prototype` object whose own prototype is the
    /// superclass's `prototype`, so instances inherit the parent's methods.
    DefineClass {
        /// Index of the constructor template in the function pool.
        function: u16,
        /// Whether a superclass value is on the stack.
        has_superclass: bool,
    },
    /// Adds a method to the class on top of the stack, leaving the class
    /// there. The method becomes a property of the class's `prototype`.
    DefineMethod {
        /// Index of the method template in the function pool.
        function: u16,
        /// Constant-pool index of the method name string.
        name: u16,
    },
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
    /// Initializes a global binding, popping its value from the stack.
    InitializeGlobal {
        /// Slot index in the global environment.
        slot: u16,
        /// Index into [`Chunk::names`](super::Chunk::names).
        name: u16,
    },
    /// Initializes a local binding, popping its value from the stack.
    InitializeLocal {
        /// Number of lexical environments between the current and target scope.
        /// It is nonzero for a `var` declared in a block, which belongs to
        /// the enclosing function's scope.
        depth: u16,
        /// Slot index in the target environment.
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
