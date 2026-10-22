//! Stack-based virtual machine for executing compiled JavaScript bytecode.

use crate::bytecode::{Chunk, Instruction};
use crate::runtime::{binary, is_truthy, unary};
use crate::{JsError, JsResult, Value};

/// Executes a [`Chunk`] using an owned operand stack.
#[derive(Debug, Default)]
pub struct Vm {
    stack: Vec<Value>,
    scopes: Vec<Vec<Slot>>,
}

#[derive(Debug, Clone)]
struct Slot {
    value: Option<Value>,
    kind: Option<crate::ast::VarKind>,
}

impl Vm {
    /// Creates a VM with an empty operand stack.
    pub fn new() -> Self {
        Self::default()
    }

    /// Executes `chunk` and returns its completion value.
    pub fn execute(&mut self, chunk: &Chunk) -> JsResult<Value> {
        self.stack.clear();
        self.scopes.clear();
        let mut instruction_pointer = 0;
        while let Some(instruction) = chunk.code.get(instruction_pointer).copied() {
            instruction_pointer += 1;
            match instruction {
                Instruction::Constant(index) => self.push(
                    chunk
                        .constants
                        .get(usize::from(index))
                        .cloned()
                        .ok_or_else(|| JsError::runtime("bytecode constant index is invalid"))?,
                ),
                Instruction::Pop => {
                    self.pop()?;
                }
                Instruction::Return => return Ok(self.stack.pop().unwrap_or(Value::Undefined)),
                Instruction::Unary(operator) => {
                    let value = self.pop()?;
                    self.push(unary(operator, &value));
                }
                Instruction::Binary(operator) => {
                    let right = self.pop()?;
                    let left = self.pop()?;
                    self.push(binary(operator, &left, &right));
                }
                Instruction::JumpIfFalse(target) => {
                    if !is_truthy(self.stack.last().ok_or_else(stack_underflow)?) {
                        instruction_pointer = target;
                    }
                }
                Instruction::JumpIfTrue(target) => {
                    if is_truthy(self.stack.last().ok_or_else(stack_underflow)?) {
                        instruction_pointer = target;
                    }
                }
                Instruction::EnterScope(slot_count) => {
                    self.scopes.push(vec![
                        Slot {
                            value: None,
                            kind: None
                        };
                        usize::from(slot_count)
                    ]);
                }
                Instruction::ExitScope => {
                    self.scopes
                        .pop()
                        .ok_or_else(|| JsError::runtime("bytecode scope underflow"))?;
                }
                Instruction::DeclareGlobal { slot, kind, name } => {
                    self.declare(0, slot, kind, name, chunk)?;
                }
                Instruction::DeclareLocal { slot, kind, name } => {
                    self.declare(0, slot, kind, name, chunk)?;
                }
                Instruction::LoadGlobal { slot, name } => {
                    let value = self.load(0, slot, name, chunk)?;
                    self.push(value);
                }
                Instruction::LoadLocal { depth, slot, name } => {
                    let value = self.load(depth, slot, name, chunk)?;
                    self.push(value);
                }
                Instruction::InitializeGlobal { slot, name } => {
                    let value = self.pop()?;
                    self.initialize(0, slot, value, name, chunk)?;
                }
                Instruction::InitializeLocal { slot, name } => {
                    let value = self.pop()?;
                    self.initialize(0, slot, value, name, chunk)?;
                }
                Instruction::StoreGlobal { slot, name } => {
                    let value = self.peek()?.clone();
                    self.store(0, slot, value, name, chunk)?;
                }
                Instruction::StoreLocal { depth, slot, name } => {
                    let value = self.peek()?.clone();
                    self.store(depth, slot, value, name, chunk)?;
                }
                Instruction::LoadUnresolved { name } => {
                    let name = self.binding_name(name, chunk)?;
                    return Err(JsError::new(format!("`{name}` is not defined")));
                }
                Instruction::StoreUnresolved { name } => {
                    let name = self.binding_name(name, chunk)?;
                    return Err(JsError::new(format!("`{name}` is not defined")));
                }
            }
        }
        Err(JsError::runtime(
            "bytecode ended without a return instruction",
        ))
    }

    fn push(&mut self, value: Value) {
        self.stack.push(value);
    }

    fn pop(&mut self) -> JsResult<Value> {
        self.stack.pop().ok_or_else(stack_underflow)
    }

    fn peek(&self) -> JsResult<&Value> {
        self.stack.last().ok_or_else(stack_underflow)
    }

    fn slot_mut(&mut self, depth: u16, slot: u16) -> JsResult<&mut Slot> {
        let index = self
            .scopes
            .len()
            .checked_sub(1 + usize::from(depth))
            .ok_or_else(|| JsError::runtime("bytecode scope depth is invalid"))?;
        self.scopes
            .get_mut(index)
            .and_then(|scope| scope.get_mut(usize::from(slot)))
            .ok_or_else(|| JsError::runtime("bytecode binding slot is invalid"))
    }

    fn slot(&self, depth: u16, slot: u16) -> JsResult<&Slot> {
        let index = self
            .scopes
            .len()
            .checked_sub(1 + usize::from(depth))
            .ok_or_else(|| JsError::runtime("bytecode scope depth is invalid"))?;
        self.scopes
            .get(index)
            .and_then(|scope| scope.get(usize::from(slot)))
            .ok_or_else(|| JsError::runtime("bytecode binding slot is invalid"))
    }

    fn binding_name<'a>(&self, name: u16, chunk: &'a Chunk) -> JsResult<&'a str> {
        chunk
            .names
            .get(usize::from(name))
            .map(String::as_str)
            .ok_or_else(|| JsError::runtime("bytecode binding name is invalid"))
    }

    fn declare(
        &mut self,
        depth: u16,
        slot: u16,
        kind: crate::ast::VarKind,
        name: u16,
        chunk: &Chunk,
    ) -> JsResult<()> {
        let name = self.binding_name(name, chunk)?.to_owned();
        let entry = self.slot_mut(depth, slot)?;
        entry.kind = Some(kind);
        if kind == crate::ast::VarKind::Var {
            entry.value = Some(Value::Undefined);
        }
        let _ = name;
        Ok(())
    }

    fn load(&self, depth: u16, slot: u16, name: u16, chunk: &Chunk) -> JsResult<Value> {
        let name = self.binding_name(name, chunk)?;
        self.slot(depth, slot)?
            .value
            .clone()
            .ok_or_else(|| JsError::new(format!("Cannot access `{name}` before initialization")))
    }

    fn initialize(
        &mut self,
        depth: u16,
        slot: u16,
        value: Value,
        name: u16,
        chunk: &Chunk,
    ) -> JsResult<()> {
        let name = self.binding_name(name, chunk)?.to_owned();
        let entry = self.slot_mut(depth, slot)?;
        if entry.value.is_some() && entry.kind == Some(crate::ast::VarKind::Const) {
            return Err(JsError::new(format!("Assignment to constant `{name}`")));
        }
        entry.value = Some(value);
        Ok(())
    }

    fn store(
        &mut self,
        depth: u16,
        slot: u16,
        value: Value,
        name: u16,
        chunk: &Chunk,
    ) -> JsResult<()> {
        let name = self.binding_name(name, chunk)?.to_owned();
        let entry = self.slot_mut(depth, slot)?;
        if entry.value.is_none() {
            return Err(JsError::new(format!(
                "Cannot access `{name}` before initialization"
            )));
        }
        if entry.kind == Some(crate::ast::VarKind::Const) {
            return Err(JsError::new(format!("Assignment to constant `{name}`")));
        }
        entry.value = Some(value);
        Ok(())
    }
}

fn stack_underflow() -> JsError {
    JsError::runtime("bytecode operand stack underflow")
}
