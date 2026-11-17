//! Lowering for the expression forms beyond plain operators: conditionals,
//! comma sequences, templates, `typeof`/`delete`, update and compound
//! assignment, optional chains, and spread.
//!
//! Every form lowers to ordinary instructions; nothing here falls back to the
//! tree-walk evaluator. Evaluation order matches the evaluator: an assignment
//! target's object and key are evaluated once, before the right operand.

use super::{CompileError, Compiler, Instruction};
use crate::Value;
use crate::ast::{
    AssignmentOperator, BinaryOperator, Expr, LogicalOperator, MemberProperty, UnaryOperator,
    UpdateOperator,
};

/// How a member reference names its property once its object (and, for a
/// computed key, the key) is on the stack.
#[derive(Clone, Copy)]
enum Key {
    /// A fixed name: the constant-pool index of the name string. Only the
    /// object is on the stack.
    Named(u16),
    /// A computed key: the object and the key are both on the stack.
    Computed,
}

impl Key {
    /// Number of stack values that identify the reference.
    fn size(self) -> u16 {
        match self {
            Self::Named(_) => 1,
            Self::Computed => 2,
        }
    }
}

fn error(message: &str) -> CompileError {
    CompileError {
        message: message.into(),
        unsupported: false,
        location: None,
    }
}

impl Compiler {
    /// Points the jump at `jump` to the next instruction to be emitted.
    fn patch_here(&mut self, jump: usize) -> Result<(), CompileError> {
        self.patch(jump)
    }

    /// Lowers `test ? consequent : alternate`.
    ///
    /// The tested value stays on the stack across the branch jump, so each
    /// branch begins by popping it.
    pub(super) fn compile_conditional(
        &mut self,
        test: &Expr,
        consequent: &Expr,
        alternate: &Expr,
    ) -> Result<(), CompileError> {
        self.compile_expression(test)?;
        let to_alternate = self.emit_placeholder(Instruction::JumpIfFalse(usize::MAX))?;
        self.chunk.emit(Instruction::Pop);
        self.compile_expression(consequent)?;
        let to_end = self.emit_placeholder(Instruction::Jump(usize::MAX))?;
        self.patch_here(to_alternate)?;
        self.chunk.emit(Instruction::Pop);
        self.compile_expression(alternate)?;
        self.patch_here(to_end)
    }

    /// Lowers `a, b, c`: every operand but the last is evaluated and popped.
    pub(super) fn compile_comma(&mut self, operands: &[Expr]) -> Result<(), CompileError> {
        let Some((last, rest)) = operands.split_last() else {
            return self.constant(Value::Undefined);
        };
        for operand in rest {
            self.compile_expression(operand)?;
            self.chunk.emit(Instruction::Pop);
        }
        self.compile_expression(last)
    }

    /// Lowers a template literal as a chain of string additions. The text so
    /// far is always a string, so each `+` converts the substitution to text.
    pub(super) fn compile_template(
        &mut self,
        quasis: &[String],
        expressions: &[Expr],
    ) -> Result<(), CompileError> {
        let mut parts = quasis.iter();
        self.constant(Value::String(parts.next().cloned().unwrap_or_default()))?;
        for (expression, quasi) in expressions.iter().zip(parts) {
            self.compile_expression(expression)?;
            self.chunk.emit(Instruction::Binary(BinaryOperator::Add));
            if !quasi.is_empty() {
                self.constant(Value::String(quasi.clone()))?;
                self.chunk.emit(Instruction::Binary(BinaryOperator::Add));
            }
        }
        Ok(())
    }

    /// Lowers a unary operator. `typeof` of a name the program never declares
    /// is the constant `"undefined"`; every other operand is evaluated first.
    pub(super) fn compile_unary(
        &mut self,
        operator: UnaryOperator,
        operand: &Expr,
    ) -> Result<(), CompileError> {
        if operator == UnaryOperator::Typeof
            && let Expr::Identifier(name) = operand
            && self.resolve(name)?.is_none()
        {
            return self.constant(Value::String("undefined".to_owned()));
        }
        self.compile_expression(operand)?;
        self.chunk.emit(Instruction::Unary(operator));
        Ok(())
    }

    /// Lowers `delete object.name` and `delete object[key]`; any other
    /// operand is evaluated for its effects and the result is `true`.
    pub(super) fn compile_delete(&mut self, operand: &Expr) -> Result<(), CompileError> {
        match operand {
            Expr::Member { object, property } => {
                self.compile_expression(object)?;
                match self.compile_key(property)? {
                    Key::Named(index) => self.chunk.emit(Instruction::DeleteProperty(index)),
                    Key::Computed => self.chunk.emit(Instruction::DeleteComputedProperty),
                };
                Ok(())
            }
            other => {
                self.compile_expression(other)?;
                self.chunk.emit(Instruction::Pop);
                self.constant(Value::Boolean(true))
            }
        }
    }

    /// Evaluates a member property's key. A computed key is pushed; a fixed
    /// name is only added to the constant pool.
    fn compile_key(&mut self, property: &MemberProperty) -> Result<Key, CompileError> {
        match property {
            MemberProperty::Named(name) => Ok(Key::Named(self.name_constant(name)?)),
            MemberProperty::Computed(key) => {
                self.compile_expression(key)?;
                Ok(Key::Computed)
            }
        }
    }

    /// Reads the property a reference on the stack names, keeping the
    /// reference beneath the value: `[object]` becomes `[object, value]` and
    /// `[object, key]` becomes `[object, key, value]`.
    fn emit_read_keeping_reference(&mut self, key: Key) {
        match key {
            Key::Named(index) => {
                self.chunk.emit(Instruction::Dup);
                self.chunk.emit(Instruction::LoadProperty(index));
            }
            Key::Computed => {
                self.chunk.emit(Instruction::Dup2);
                self.chunk.emit(Instruction::LoadComputedProperty);
            }
        }
    }

    /// Stores the value on top into the referenced property, replacing the
    /// reference and the value with the stored value.
    fn emit_write(&mut self, key: Key) {
        match key {
            Key::Named(index) => self.chunk.emit(Instruction::StoreProperty(index)),
            Key::Computed => self.chunk.emit(Instruction::StoreComputedProperty),
        };
    }

    /// Lowers `++x`, `x++`, `--x`, and `x--`.
    ///
    /// The old value is converted with unary plus first, so `'5'++` style
    /// strings become numbers and a postfix update yields that number.
    pub(super) fn compile_update(
        &mut self,
        target: &Expr,
        operator: UpdateOperator,
        prefix: bool,
    ) -> Result<(), CompileError> {
        let step = match operator {
            UpdateOperator::Increment => BinaryOperator::Add,
            UpdateOperator::Decrement => BinaryOperator::Subtract,
        };
        match target {
            Expr::Identifier(name) => {
                self.compile_load(name)?;
                self.chunk.emit(Instruction::Unary(UnaryOperator::Plus));
                if !prefix {
                    self.chunk.emit(Instruction::Dup);
                }
                self.constant(Value::Number(1.0))?;
                self.chunk.emit(Instruction::Binary(step));
                self.compile_store(name)?;
                if !prefix {
                    self.chunk.emit(Instruction::Pop);
                }
                Ok(())
            }
            Expr::Member { object, property } => {
                self.compile_expression(object)?;
                let key = self.compile_key(property)?;
                self.emit_read_keeping_reference(key);
                self.chunk.emit(Instruction::Unary(UnaryOperator::Plus));
                if !prefix {
                    // Leave the old value beneath the reference for the result.
                    self.chunk.emit(Instruction::Dup);
                    self.chunk.emit(Instruction::MoveTop(key.size() + 1));
                }
                self.constant(Value::Number(1.0))?;
                self.chunk.emit(Instruction::Binary(step));
                self.emit_write(key);
                if !prefix {
                    self.chunk.emit(Instruction::Pop);
                }
                Ok(())
            }
            _ => Err(error("invalid update target")),
        }
    }

    /// Emits the test that decides whether a logical operator skips its
    /// right operand, with the left value left on the stack. Returns the
    /// jump to patch to the skip target.
    ///
    /// `&&` skips when the value is falsy, `||` when it is truthy, and `??`
    /// when it is neither `null` nor `undefined`.
    pub(super) fn emit_logical_skip(
        &mut self,
        operator: LogicalOperator,
    ) -> Result<usize, CompileError> {
        match operator {
            LogicalOperator::And => self.emit_placeholder(Instruction::JumpIfFalse(usize::MAX)),
            LogicalOperator::Or => self.emit_placeholder(Instruction::JumpIfTrue(usize::MAX)),
            LogicalOperator::Nullish => {
                let to_right = self.emit_placeholder(Instruction::JumpIfNullish(usize::MAX))?;
                let skip = self.emit_placeholder(Instruction::Jump(usize::MAX))?;
                self.patch_here(to_right)?;
                Ok(skip)
            }
        }
    }

    /// Lowers `a && b`, `a || b`, and `a ?? b`.
    pub(super) fn compile_logical(
        &mut self,
        left: &Expr,
        operator: LogicalOperator,
        right: &Expr,
    ) -> Result<(), CompileError> {
        self.compile_expression(left)?;
        let skip = self.emit_logical_skip(operator)?;
        self.chunk.emit(Instruction::Pop);
        self.compile_expression(right)?;
        self.patch_here(skip)
    }

    /// Lowers compound assignment. The target's object and key are evaluated
    /// once; a logical operator that leaves the target alone also skips the
    /// right operand and the store.
    pub(super) fn compile_compound(
        &mut self,
        target: &Expr,
        operator: AssignmentOperator,
        value: &Expr,
    ) -> Result<(), CompileError> {
        match target {
            Expr::Identifier(name) => {
                self.compile_load(name)?;
                match operator {
                    AssignmentOperator::Binary(operator) => {
                        self.compile_expression(value)?;
                        self.chunk.emit(Instruction::Binary(operator));
                        self.compile_store(name)
                    }
                    AssignmentOperator::Logical(operator) => {
                        let skip = self.emit_logical_skip(operator)?;
                        self.chunk.emit(Instruction::Pop);
                        self.compile_expression(value)?;
                        self.compile_store(name)?;
                        self.patch_here(skip)
                    }
                }
            }
            Expr::Member { object, property } => {
                self.compile_expression(object)?;
                let key = self.compile_key(property)?;
                self.emit_read_keeping_reference(key);
                match operator {
                    AssignmentOperator::Binary(operator) => {
                        self.compile_expression(value)?;
                        self.chunk.emit(Instruction::Binary(operator));
                        self.emit_write(key);
                        Ok(())
                    }
                    AssignmentOperator::Logical(operator) => {
                        let skip = self.emit_logical_skip(operator)?;
                        self.chunk.emit(Instruction::Pop);
                        self.compile_expression(value)?;
                        self.emit_write(key);
                        let to_end = self.emit_placeholder(Instruction::Jump(usize::MAX))?;
                        // Skipped: `[reference.., old]` collapses to `[old]`.
                        self.patch_here(skip)?;
                        match key {
                            Key::Named(_) => {
                                self.chunk.emit(Instruction::Swap);
                                self.chunk.emit(Instruction::Pop);
                            }
                            Key::Computed => {
                                self.chunk.emit(Instruction::MoveTop(2));
                                self.chunk.emit(Instruction::Pop);
                                self.chunk.emit(Instruction::Pop);
                            }
                        }
                        self.patch_here(to_end)
                    }
                }
            }
            _ => Err(error("invalid assignment target")),
        }
    }

    /// Records a jump taken when the value on top is `null` or `undefined`,
    /// to be landed at the end of the innermost optional chain. `extra` is how
    /// many values the chain keeps beneath that value (the receiver of a
    /// method call).
    fn emit_optional_exit(&mut self, extra: usize) -> Result<(), CompileError> {
        let jump = self.emit_placeholder(Instruction::JumpIfNullish(usize::MAX))?;
        self.optional_exits
            .last_mut()
            .ok_or_else(|| error("optional access outside an optional chain"))?
            .push((jump, extra));
        Ok(())
    }

    /// Lowers an optional chain. Each `?.` inside it jumps to a landing pad
    /// that discards what the chain had pushed and produces `undefined`.
    pub(super) fn compile_optional_chain(&mut self, chain: &Expr) -> Result<(), CompileError> {
        self.optional_exits.push(Vec::new());
        let result = self.compile_expression(chain);
        let exits = self.optional_exits.pop().unwrap_or_default();
        result?;
        if exits.is_empty() {
            return Ok(());
        }
        let mut to_end = vec![self.emit_placeholder(Instruction::Jump(usize::MAX))?];
        let mut pads: Vec<(usize, usize)> = Vec::new();
        for (jump, extra) in exits {
            let start = match pads.iter().find(|(known, _)| *known == extra) {
                Some((_, start)) => *start,
                None => {
                    let start = self.chunk.current_offset();
                    for _ in 0..=extra {
                        self.chunk.emit(Instruction::Pop);
                    }
                    self.constant(Value::Undefined)?;
                    to_end.push(self.emit_placeholder(Instruction::Jump(usize::MAX))?);
                    pads.push((extra, start));
                    start
                }
            };
            self.patch_to(jump, start)?;
        }
        for jump in to_end {
            self.patch_here(jump)?;
        }
        Ok(())
    }

    /// Lowers `object?.name` and `object?.[key]` inside a chain.
    pub(super) fn compile_optional_member(
        &mut self,
        object: &Expr,
        property: &MemberProperty,
    ) -> Result<(), CompileError> {
        self.compile_expression(object)?;
        self.emit_optional_exit(0)?;
        self.emit_load_property(property)
    }

    /// Loads the property of the object on top of the stack.
    pub(super) fn emit_load_property(
        &mut self,
        property: &MemberProperty,
    ) -> Result<(), CompileError> {
        match property {
            MemberProperty::Named(name) => {
                let index = self.name_constant(name)?;
                self.chunk.emit(Instruction::LoadProperty(index));
            }
            MemberProperty::Computed(key) => {
                self.compile_expression(key)?;
                self.chunk.emit(Instruction::LoadComputedProperty);
            }
        }
        Ok(())
    }

    /// Pushes a callee for a call. Returns whether a receiver was pushed
    /// beneath the function, which makes the call a method call.
    fn compile_callee(&mut self, callee: &Expr) -> Result<bool, CompileError> {
        match callee {
            Expr::Member { object, property } => {
                self.compile_expression(object)?;
                self.chunk.emit(Instruction::Dup);
                self.emit_load_property(property)?;
                Ok(true)
            }
            Expr::OptionalMember { object, property } => {
                self.compile_expression(object)?;
                self.emit_optional_exit(0)?;
                self.chunk.emit(Instruction::Dup);
                self.emit_load_property(property)?;
                Ok(true)
            }
            Expr::SuperMember { name } => {
                self.require_super()?;
                self.chunk.emit(Instruction::LoadThis);
                let index = self.name_constant(name)?;
                self.chunk.emit(Instruction::LoadSuperProperty(index));
                Ok(true)
            }
            other => {
                self.compile_expression(other)?;
                Ok(false)
            }
        }
    }

    /// Lowers a call. A member callee is a method call: the object is kept
    /// below the function so it becomes `this`.
    pub(super) fn compile_call(
        &mut self,
        callee: &Expr,
        arguments: &[Expr],
    ) -> Result<(), CompileError> {
        let method = self.compile_callee(callee)?;
        self.emit_call(method, arguments)
    }

    /// Lowers `callee?.(arguments)`: a `null` or `undefined` function ends the
    /// chain before any argument is evaluated.
    pub(super) fn compile_optional_call(
        &mut self,
        callee: &Expr,
        arguments: &[Expr],
    ) -> Result<(), CompileError> {
        let method = self.compile_callee(callee)?;
        self.emit_optional_exit(usize::from(method))?;
        self.emit_call(method, arguments)
    }

    /// Emits the arguments and the call instruction for a callee already on
    /// the stack. Spread arguments are gathered into one array first.
    fn emit_call(&mut self, method: bool, arguments: &[Expr]) -> Result<(), CompileError> {
        if has_spread(arguments) {
            self.compile_argument_array(arguments)?;
            self.chunk.emit(if method {
                Instruction::CallMethodSpread
            } else {
                Instruction::CallSpread
            });
        } else {
            let count = self.compile_arguments(arguments)?;
            self.chunk.emit(if method {
                Instruction::CallMethod(count)
            } else {
                Instruction::Call(count)
            });
        }
        Ok(())
    }

    /// Lowers `new Callee(arguments)`.
    pub(super) fn compile_new(
        &mut self,
        callee: &Expr,
        arguments: &[Expr],
    ) -> Result<(), CompileError> {
        self.compile_expression(callee)?;
        if has_spread(arguments) {
            self.compile_argument_array(arguments)?;
            self.chunk.emit(Instruction::NewSpread);
        } else {
            let count = self.compile_arguments(arguments)?;
            self.chunk.emit(Instruction::New(count));
        }
        Ok(())
    }

    /// Builds an array from call arguments, one push or extend each.
    fn compile_argument_array(&mut self, arguments: &[Expr]) -> Result<(), CompileError> {
        let parts: Vec<_> = arguments.iter().map(Some).collect();
        self.compile_spread_array(&parts)
    }

    /// Lowers an array literal. Without spread each element is pushed and
    /// one `NewArray` collects them; with spread the array is built
    /// incrementally.
    pub(super) fn compile_array(&mut self, elements: &[Option<Expr>]) -> Result<(), CompileError> {
        if elements
            .iter()
            .flatten()
            .any(|element| matches!(element, Expr::Spread(_)))
        {
            let parts: Vec<_> = elements.iter().map(Option::as_ref).collect();
            return self.compile_spread_array(&parts);
        }
        for element in elements {
            match element {
                Some(element) => self.compile_expression(element)?,
                None => self.constant(Value::Undefined)?,
            }
        }
        let count = u16::try_from(elements.len())
            .map_err(|_| CompileError::unsupported("array literal with too many elements"))?;
        self.chunk.emit(Instruction::NewArray(count));
        Ok(())
    }

    /// Builds an array element by element: `None` is an elision.
    fn compile_spread_array(&mut self, parts: &[Option<&Expr>]) -> Result<(), CompileError> {
        self.chunk.emit(Instruction::NewArray(0));
        for part in parts {
            match part {
                Some(Expr::Spread(source)) => {
                    self.compile_expression(source)?;
                    self.chunk.emit(Instruction::ArrayExtend);
                }
                Some(element) => {
                    self.compile_expression(element)?;
                    self.chunk.emit(Instruction::ArrayPush);
                }
                None => {
                    self.constant(Value::Undefined)?;
                    self.chunk.emit(Instruction::ArrayPush);
                }
            }
        }
        Ok(())
    }
}

/// Whether any argument is a spread, which needs the array-based calls.
fn has_spread(arguments: &[Expr]) -> bool {
    arguments
        .iter()
        .any(|argument| matches!(argument, Expr::Spread(_)))
}
