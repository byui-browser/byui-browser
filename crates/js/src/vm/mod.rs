//! Stack-based virtual machine for executing compiled JavaScript bytecode.

use crate::bytecode::{Chunk, Instruction};
use crate::runtime::{binary, is_truthy, unary};
use crate::{JsError, JsResult, Value};

/// Executes a [`Chunk`] using an owned operand stack.
#[derive(Debug, Default)]
pub struct Vm {
    stack: Vec<Value>,
}

impl Vm {
    /// Creates a VM with an empty operand stack.
    pub fn new() -> Self {
        Self::default()
    }

    /// Executes `chunk` and returns its completion value.
    pub fn execute(&mut self, chunk: &Chunk) -> JsResult<Value> {
        self.stack.clear();
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
}

fn stack_underflow() -> JsError {
    JsError::runtime("bytecode operand stack underflow")
}
