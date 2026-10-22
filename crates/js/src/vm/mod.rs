//! Stack-based virtual machine for executing compiled JavaScript bytecode.

use crate::ast::{BinaryOperator, UnaryOperator};
use crate::bytecode::{Chunk, Instruction};
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
                    self.push(unary(operator, value)?);
                }
                Instruction::Binary(operator) => {
                    let right = self.pop()?;
                    let left = self.pop()?;
                    self.push(binary(operator, left, right)?);
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

fn unary(operator: UnaryOperator, value: Value) -> JsResult<Value> {
    match operator {
        UnaryOperator::Negate => Ok(Value::Number(-to_number(&value)?)),
        UnaryOperator::Not => Ok(Value::Boolean(!is_truthy(&value))),
        UnaryOperator::BitwiseNot => Ok(Value::Number(f64::from(!to_int32(&value)?))),
    }
}

fn binary(operator: BinaryOperator, left: Value, right: Value) -> JsResult<Value> {
    match operator {
        BinaryOperator::Add => match (left, right) {
            (Value::String(left), right) => Ok(Value::String(left + &right.to_string())),
            (left, Value::String(right)) => Ok(Value::String(left.to_string() + &right)),
            (left, right) => Ok(Value::Number(to_number(&left)? + to_number(&right)?)),
        },
        BinaryOperator::Subtract => number_binary(left, right, |a, b| a - b),
        BinaryOperator::Multiply => number_binary(left, right, |a, b| a * b),
        BinaryOperator::Divide => number_binary(left, right, |a, b| a / b),
        BinaryOperator::Remainder => number_binary(left, right, |a, b| a % b),
        BinaryOperator::BitwiseAnd => int_binary(left, right, |a, b| a & b),
        BinaryOperator::BitwiseOr => int_binary(left, right, |a, b| a | b),
        BinaryOperator::BitwiseXor => int_binary(left, right, |a, b| a ^ b),
        BinaryOperator::LeftShift => {
            int_binary(left, right, |a, b| a.wrapping_shl((b as u32) & 31))
        }
        BinaryOperator::SignedRightShift => int_binary(left, right, |a, b| a >> ((b as u32) & 31)),
        BinaryOperator::UnsignedRightShift => Ok(Value::Number(f64::from(
            (to_int32(&left)? as u32).wrapping_shr((to_uint32(&right)? as u32) & 31),
        ))),
        BinaryOperator::Less => Ok(Value::Boolean(to_number(&left)? < to_number(&right)?)),
        BinaryOperator::LessEqual => Ok(Value::Boolean(to_number(&left)? <= to_number(&right)?)),
        BinaryOperator::Greater => Ok(Value::Boolean(to_number(&left)? > to_number(&right)?)),
        BinaryOperator::GreaterEqual => Ok(Value::Boolean(to_number(&left)? >= to_number(&right)?)),
        BinaryOperator::StrictEqual => Ok(Value::Boolean(left == right)),
        BinaryOperator::StrictNotEqual => Ok(Value::Boolean(left != right)),
        BinaryOperator::Equal => Ok(Value::Boolean(left == right)),
        BinaryOperator::NotEqual => Ok(Value::Boolean(left != right)),
    }
}

fn number_binary(
    left: Value,
    right: Value,
    operation: impl FnOnce(f64, f64) -> f64,
) -> JsResult<Value> {
    Ok(Value::Number(operation(
        to_number(&left)?,
        to_number(&right)?,
    )))
}

fn int_binary(
    left: Value,
    right: Value,
    operation: impl FnOnce(i32, i32) -> i32,
) -> JsResult<Value> {
    Ok(Value::Number(f64::from(operation(
        to_int32(&left)?,
        to_int32(&right)?,
    ))))
}

fn to_number(value: &Value) -> JsResult<f64> {
    match value {
        Value::Number(value) => Ok(*value),
        Value::Boolean(value) => Ok(f64::from(*value as u8)),
        Value::Null => Ok(0.0),
        Value::Undefined => Ok(f64::NAN),
        Value::String(value) if value.trim().is_empty() => Ok(0.0),
        Value::String(value) => Ok(value.trim().parse().unwrap_or(f64::NAN)),
        Value::Function(_) | Value::Object(_) => {
            Err(JsError::runtime("cannot convert object to number"))
        }
    }
}

fn to_int32(value: &Value) -> JsResult<i32> {
    Ok(to_number(value)? as i32)
}

fn to_uint32(value: &Value) -> JsResult<u32> {
    Ok(to_number(value)? as u32)
}

fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Undefined | Value::Null => false,
        Value::Boolean(value) => *value,
        Value::Number(value) => *value != 0.0 && !value.is_nan(),
        Value::String(value) => !value.is_empty(),
        Value::Function(_) | Value::Object(_) => true,
    }
}
