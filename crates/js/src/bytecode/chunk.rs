//! Storage for bytecode instructions and their constants.

use super::Instruction;
use crate::Value;

/// A source position associated with a bytecode instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceLocation {
    /// Zero-based UTF-8 byte offset in the source text.
    pub offset: usize,
    /// One-based source line.
    pub line: usize,
    /// One-based character column.
    pub column: usize,
}

/// An owned sequence of instructions and the values referenced by them.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    /// Instructions in execution order.
    pub code: Vec<Instruction>,
    /// Constants addressed by [`Instruction::Constant`] operands.
    pub constants: Vec<Value>,
    /// Binding names used to produce useful VM errors.
    pub names: Vec<String>,
    /// Optional source location for each instruction, indexed like [`Self::code`].
    pub locations: Vec<Option<SourceLocation>>,
}

impl Chunk {
    /// Creates an empty chunk.
    pub fn new() -> Self {
        Self {
            code: Vec::new(),
            constants: Vec::new(),
            names: Vec::new(),
            locations: Vec::new(),
        }
    }

    /// Appends an instruction and returns its instruction offset.
    pub fn emit(&mut self, instruction: Instruction) -> usize {
        self.emit_at(instruction, None)
    }

    /// Appends an instruction with an optional source location.
    pub fn emit_at(&mut self, instruction: Instruction, location: Option<SourceLocation>) -> usize {
        let offset = self.code.len();
        self.code.push(instruction);
        self.locations.push(location);
        offset
    }

    /// Returns the source location for an instruction, if one was recorded.
    pub fn location(&self, offset: usize) -> Option<SourceLocation> {
        self.locations.get(offset).copied().flatten()
    }

    /// Renders this chunk in a deterministic, human-readable format.
    pub fn disassemble(&self) -> String {
        self.disassemble_named("script")
    }

    /// Renders this chunk with a caller-provided function/chunk name.
    pub fn disassemble_named(&self, name: &str) -> String {
        use std::fmt::Write;
        let mut output = format!("== {name} ==\n");
        for (offset, instruction) in self.code.iter().enumerate() {
            let location = self
                .location(offset)
                .map(|position| format!("{}:{} ", position.line, position.column))
                .unwrap_or_default();
            let _ = writeln!(
                output,
                "{offset:04} {location}{}",
                self.format_instruction(instruction)
            );
        }
        output
    }

    fn format_instruction(&self, instruction: &Instruction) -> String {
        match instruction {
            Instruction::Constant(index) => match self.constants.get(usize::from(*index)) {
                Some(value) => format!("CONSTANT {index} ({value:?})"),
                None => format!("CONSTANT {index} (<invalid>)"),
            },
            Instruction::Jump(target) => format!("JUMP -> {target:04}"),
            Instruction::JumpIfFalse(target) => format!("JUMP_IF_FALSE -> {target:04}"),
            Instruction::JumpIfTrue(target) => format!("JUMP_IF_TRUE -> {target:04}"),
            Instruction::DeclareGlobal { slot, name, .. }
            | Instruction::DeclareLocal { slot, name, .. }
            | Instruction::InitializeGlobal { slot, name }
            | Instruction::InitializeLocal { slot, name }
            | Instruction::StoreGlobal { slot, name }
            | Instruction::StoreLocal { slot, name, .. }
            | Instruction::LoadGlobal { slot, name }
            | Instruction::LoadLocal { slot, name, .. } => {
                let binding = self
                    .names
                    .get(usize::from(*name))
                    .map(String::as_str)
                    .unwrap_or("<invalid>");
                format!("{instruction:?} ({binding:?}, slot {slot})")
            }
            Instruction::LoadUnresolved { name } | Instruction::StoreUnresolved { name } => {
                let binding = self
                    .names
                    .get(usize::from(*name))
                    .map(String::as_str)
                    .unwrap_or("<invalid>");
                format!("{instruction:?} ({binding:?})")
            }
            other => format!("{other:?}"),
        }
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
