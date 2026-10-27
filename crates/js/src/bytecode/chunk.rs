//! Storage for bytecode instructions and their constants.

use super::Instruction;
use crate::Value;
use std::sync::Arc;

/// An owned sequence of instructions and the values referenced by them.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    /// Instructions in execution order.
    pub code: Vec<Instruction>,
    /// Constants addressed by [`Instruction::Constant`] operands.
    pub constants: Vec<Value>,
    /// Binding names used to produce useful VM errors.
    pub names: Vec<String>,
    /// Compiled functions addressed by [`Instruction::MakeFunction`],
    /// [`Instruction::DefineClass`], and [`Instruction::DefineMethod`].
    pub functions: Vec<Arc<FunctionTemplate>>,
}

/// How a compiled function behaves when called or constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallableKind {
    /// A `function` declaration: callable, and constructible with `new`.
    Function,
    /// A class method: callable with a receiver, never constructible.
    Method,
    /// A class constructor with no superclass. It must be run with `new`.
    BaseConstructor,
    /// A constructor of a class with a superclass. It must be run with
    /// `new`, and `this` exists only after `super(...)` returns.
    DerivedConstructor,
    /// The implicit constructor of a derived class without its own
    /// `constructor`: it forwards every argument to the parent constructor.
    DefaultDerivedConstructor,
}

impl CallableKind {
    /// Whether `new` may run a function of this kind.
    pub fn is_constructor(self) -> bool {
        !matches!(self, Self::Method)
    }

    /// Whether this kind is a class constructor.
    pub fn is_class(self) -> bool {
        matches!(
            self,
            Self::BaseConstructor | Self::DerivedConstructor | Self::DefaultDerivedConstructor
        )
    }
}

/// The compiled, shareable form of one function or method body.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionTemplate {
    /// The name shown in diagnostics; a class constructor uses the class name.
    pub name: String,
    /// How the function is called and constructed.
    pub kind: CallableKind,
    /// Number of declared parameters. Extra arguments are ignored and
    /// missing ones are `undefined`.
    pub param_count: u16,
    /// The function body, shared by every function value made from this
    /// template. Its [`Chunk::functions`] hold nested templates.
    pub chunk: Arc<Chunk>,
}

impl Chunk {
    /// Creates an empty chunk.
    pub fn new() -> Self {
        Self {
            code: Vec::new(),
            constants: Vec::new(),
            names: Vec::new(),
            functions: Vec::new(),
        }
    }

    /// Adds a compiled function and returns its pool index.
    pub fn add_function(&mut self, template: FunctionTemplate) -> Result<u16, &'static str> {
        let index = u16::try_from(self.functions.len()).map_err(|_| "function pool is full")?;
        self.functions.push(Arc::new(template));
        Ok(index)
    }

    /// Appends an instruction and returns its instruction offset.
    pub fn emit(&mut self, instruction: Instruction) -> usize {
        let offset = self.code.len();
        self.code.push(instruction);
        offset
    }

    /// Returns the offset where the next instruction will be emitted.
    pub fn current_offset(&self) -> usize {
        self.code.len()
    }

    /// Emits a jump with a placeholder target and returns its offset.
    pub fn emit_jump(&mut self, instruction: Instruction) -> Result<usize, &'static str> {
        if !matches!(
            instruction,
            Instruction::Jump(_) | Instruction::JumpIfFalse(_) | Instruction::JumpIfTrue(_)
        ) {
            return Err("instruction is not patchable");
        }
        Ok(self.emit(instruction))
    }

    /// Replaces a placeholder jump target with a valid instruction offset.
    pub fn patch_jump(&mut self, offset: usize, target: usize) -> Result<(), &'static str> {
        if target > self.code.len() {
            return Err("jump target is outside the chunk");
        }
        let instruction = self.code.get_mut(offset).ok_or("jump offset is invalid")?;
        match instruction {
            Instruction::Jump(target_slot)
            | Instruction::JumpIfFalse(target_slot)
            | Instruction::JumpIfTrue(target_slot) => *target_slot = target,
            _ => return Err("instruction is not patchable"),
        }
        Ok(())
    }

    /// Adds an owned constant and returns its pool index.
    pub fn add_constant(&mut self, value: Value) -> Result<u16, &'static str> {
        let index = u16::try_from(self.constants.len()).map_err(|_| "constant pool is full")?;
        self.constants.push(value);
        Ok(index)
    }

    /// Adds a binding name and returns its pool index.
    pub fn add_name(&mut self, name: &str) -> Result<u16, &'static str> {
        if let Some(index) = self.names.iter().position(|entry| entry == name) {
            return u16::try_from(index).map_err(|_| "binding name pool is full");
        }
        let index = u16::try_from(self.names.len()).map_err(|_| "binding name pool is full")?;
        self.names.push(name.to_owned());
        Ok(index)
    }
}

impl Default for Chunk {
    fn default() -> Self {
        Self::new()
    }
}
