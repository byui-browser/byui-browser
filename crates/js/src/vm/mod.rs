//! Stack-based virtual machine for executing compiled JavaScript bytecode.

use crate::ast::{BinaryOperator, UnaryOperator};
use crate::bytecode::{Chunk, Instruction};
use crate::runtime::{ExecutionLimits, non_object, to_property_key};
use crate::{JsError, JsResult, Object, Value};

/// Executes a [`Chunk`] using an owned operand stack.
#[derive(Debug, Default)]
pub struct Vm {
    stack: Vec<Value>,
    scopes: Vec<Vec<Slot>>,
    steps_remaining: u64,
    loop_iterations_remaining: u64,
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
        self.execute_with_limits(chunk, ExecutionLimits::default())
    }

    /// Executes `chunk` with caller-provided instruction and loop limits.
    ///
    /// One step is charged for every instruction. A loop iteration is charged
    /// when execution takes a backward jump, which keeps an infinite loop
    /// bounded even when its condition and body are otherwise inexpensive.
    pub fn execute_with_limits(
        &mut self,
        chunk: &Chunk,
        limits: ExecutionLimits,
    ) -> JsResult<Value> {
        self.stack.clear();
        self.scopes.clear();
        self.steps_remaining = limits.step_limit;
        self.loop_iterations_remaining = limits.loop_iteration_limit;
        let mut instruction_pointer = 0;
        while let Some(instruction) = chunk.code.get(instruction_pointer).copied() {
            if self.steps_remaining == 0 {
                return Err(JsError::with_context(
                    crate::JsErrorCategory::Limit,
                    "Script exceeded the evaluation step limit",
                    "bytecode instruction budget",
                ));
            }
            self.steps_remaining -= 1;
            instruction_pointer += 1;
            match instruction {
                Instruction::Constant(index) => self.push(
                    chunk
                        .constants
                        .get(usize::from(index))
                        .cloned()
                        .ok_or_else(|| {
                            bytecode_error(
                                chunk,
                                instruction_pointer - 1,
                                "bytecode constant index is invalid",
                            )
                        })?,
                ),
                Instruction::Pop => {
                    self.pop()?;
                }
                Instruction::Swap => {
                    let right = self.pop()?;
                    let left = self.pop()?;
                    self.push(right);
                    self.push(left);
                }
                Instruction::Jump(target) => {
                    validate_jump(target, chunk, instruction_pointer - 1)?;
                    if target < instruction_pointer {
                        self.next_loop_iteration()?;
                    }
                    instruction_pointer = target;
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
                    validate_jump(target, chunk, instruction_pointer - 1)?;
                    if !is_truthy(self.stack.last().ok_or_else(stack_underflow)?) {
                        instruction_pointer = target;
                    }
                }
                Instruction::JumpIfTrue(target) => {
                    validate_jump(target, chunk, instruction_pointer - 1)?;
                    if is_truthy(self.stack.last().ok_or_else(stack_underflow)?) {
                        instruction_pointer = target;
                    }
                }
                Instruction::LoadProperty(index) => {
                    let key = property_name(chunk, index)?;
                    let target = self.pop()?;
                    self.push(load_property(&target, &key)?);
                }
                Instruction::LoadComputedProperty => {
                    let key = to_property_key(&self.pop()?);
                    let target = self.pop()?;
                    self.push(load_property(&target, &key)?);
                }
                Instruction::StoreProperty(index) => {
                    let key = property_name(chunk, index)?;
                    let value = self.pop()?;
                    let target = self.pop()?;
                    store_property(&target, key, &value)?;
                    self.push(value);
                }
                Instruction::StoreComputedProperty => {
                    let value = self.pop()?;
                    let key = to_property_key(&self.pop()?);
                    let target = self.pop()?;
                    store_property(&target, key, &value)?;
                    self.push(value);
                }
                Instruction::NewObject => self.push(Value::Object(Object::new())),
                Instruction::DefineProperty(index) => {
                    let key = property_name(chunk, index)?;
                    let value = self.pop()?;
                    store_property(self.peek()?, key, &value)?;
                }
                Instruction::DefineComputedProperty => {
                    let value = self.pop()?;
                    let key = to_property_key(&self.pop()?);
                    store_property(self.peek()?, key, &value)?;
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
                    self.declare(self.global_depth()?, slot, kind, name, chunk)?;
                }
                Instruction::DeclareLocal { slot, kind, name } => {
                    self.declare(0, slot, kind, name, chunk)?;
                }
                Instruction::LoadGlobal { slot, name } => {
                    let value = self.load(self.global_depth()?, slot, name, chunk)?;
                    self.push(value);
                }
                Instruction::LoadLocal { depth, slot, name } => {
                    let value = self.load(depth, slot, name, chunk)?;
                    self.push(value);
                }
                Instruction::InitializeGlobal { slot, name } => {
                    let value = self.pop()?;
                    self.initialize(self.global_depth()?, slot, value, name, chunk)?;
                }
                Instruction::InitializeLocal { slot, name } => {
                    let value = self.pop()?;
                    self.initialize(0, slot, value, name, chunk)?;
                }
                Instruction::StoreGlobal { slot, name } => {
                    let value = self.peek()?.clone();
                    self.store(self.global_depth()?, slot, value, name, chunk)?;
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

    fn next_loop_iteration(&mut self) -> JsResult<()> {
        if self.loop_iterations_remaining == 0 {
            return Err(JsError::with_context(
                crate::JsErrorCategory::Limit,
                "Script exceeded the loop iteration limit",
                "bytecode loop iteration",
            ));
        }
        self.loop_iterations_remaining -= 1;
        Ok(())
    }

    fn global_depth(&self) -> JsResult<u16> {
        let depth = self
            .scopes
            .len()
            .checked_sub(1)
            .ok_or_else(|| JsError::runtime("bytecode global scope is unavailable"))?;
        u16::try_from(depth).map_err(|_| JsError::runtime("bytecode scope depth is invalid"))
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

fn validate_jump(target: usize, chunk: &Chunk, offset: usize) -> JsResult<()> {
    if target > chunk.code.len() {
        return Err(bytecode_error(
            chunk,
            offset,
            "bytecode jump target is invalid",
        ));
    }
    Ok(())
}

fn stack_underflow() -> JsError {
    JsError::runtime("bytecode operand stack underflow")
}

fn bytecode_error(chunk: &Chunk, offset: usize, message: &'static str) -> JsError {
    let error = JsError::runtime(message);
    chunk
        .location(offset)
        .map_or(error.clone(), |location| error.with_location(location))
}

/// Reads the string constant naming a property.
fn property_name(chunk: &Chunk, index: u16) -> JsResult<String> {
    match chunk.constants.get(usize::from(index)) {
        Some(Value::String(name)) => Ok(name.clone()),
        _ => Err(JsError::runtime(
            "bytecode property name constant is invalid",
        )),
    }
}

fn load_property(target: &Value, key: &str) -> JsResult<Value> {
    let Value::Object(object) = target else {
        return Err(non_object(target, "read", key, "property access"));
    };
    Ok(object.get(key).unwrap_or(Value::Undefined))
}

fn store_property(target: &Value, key: String, value: &Value) -> JsResult<()> {
    let Value::Object(object) = target else {
        return Err(non_object(target, "set", &key, "property assignment"));
    };
    object.set(key, value.clone());
    Ok(())
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
