//! Storage for bytecode instructions and their constants.

use super::Instruction;
use crate::Value;

/// An owned sequence of instructions and the values referenced by them.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    /// Instructions in execution order.
    pub code: Vec<Instruction>,
    /// Constants addressed by [`Instruction::Constant`] operands.
    pub constants: Vec<Value>,
    /// Binding names used to produce useful VM errors.
    pub names: Vec<String>,
}

impl Chunk {
    /// Creates an empty chunk.
    pub fn new() -> Self {
        Self {
            code: Vec::new(),
            constants: Vec::new(),
            names: Vec::new(),
        }
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
