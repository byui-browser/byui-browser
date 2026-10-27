//! AST-to-bytecode compiler.

use super::{Chunk, Instruction};
use crate::Value;
use crate::ast::{Expr, LogicalOperator, MemberProperty, Program, Statement, VarKind};
use std::collections::HashMap;
use std::fmt;

#[path = "control_flow.rs"]
mod control_flow;

/// A source construct that the initial bytecode compiler cannot lower yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    /// Human-readable description of the unsupported construct.
    pub message: String,
    /// Whether the error means the compiler does not lower this syntax yet.
    pub unsupported: bool,
}

impl CompileError {
    fn unsupported(construct: &str) -> Self {
        Self {
            message: format!("bytecode compiler does not support {construct} yet"),
            unsupported: true,
        }
    }

    /// Returns whether compilation rejected syntax that has not been lowered.
    pub fn is_unsupported(&self) -> bool {
        self.unsupported
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CompileError {}

/// Compiles supported expressions and variable bindings into a fresh [`Chunk`].
#[derive(Debug)]
pub struct Compiler {
    chunk: Chunk,
    scopes: Vec<Scope>,
}

#[derive(Debug)]
struct Scope {
    bindings: HashMap<String, Binding>,
    slots: u16,
}

#[derive(Debug, Clone, Copy)]
struct Binding {
    scope: usize,
    slot: u16,
    kind: VarKind,
}

impl Compiler {
    /// Creates a compiler with an empty constant pool and instruction stream.
    pub fn new() -> Self {
        Self {
            chunk: Chunk::new(),
            scopes: vec![Scope {
                bindings: HashMap::new(),
                slots: 0,
            }],
        }
    }

    /// Compiles `program`, returning an owned chunk ready for [`crate::vm::Vm`].
    pub fn compile(mut self, program: &Program) -> Result<Chunk, CompileError> {
        self.collect_root_bindings(program)?;
        let root_enter = self.chunk.emit(Instruction::EnterScope(0));
        self.emit_scope_declarations(0)?;
        self.compile_sequence(&program.body, true)?;
        self.chunk.code[root_enter] = Instruction::EnterScope(self.scopes[0].slots);
        self.chunk.emit(Instruction::Return);
        Ok(self.chunk)
    }

    fn collect_root_bindings(&mut self, program: &Program) -> Result<(), CompileError> {
        for statement in &program.body {
            if let Statement::VariableDeclaration { kind, name, .. } = statement {
                if *kind != VarKind::Var {
                    self.add_binding(0, name, *kind)?;
                }
            }
            self.collect_vars(statement)?;
        }
        Ok(())
    }

    fn collect_vars(&mut self, statement: &Statement) -> Result<(), CompileError> {
        match statement {
            Statement::VariableDeclaration {
                kind: VarKind::Var,
                name,
                ..
            } => {
                self.add_binding(0, name, VarKind::Var)?;
            }
            Statement::Block(statements) => {
                for statement in statements {
                    self.collect_vars(statement)?;
                }
            }
            Statement::If {
                then_branch,
                else_branch,
                ..
            } => {
                self.collect_vars(then_branch)?;
                if let Some(branch) = else_branch {
                    self.collect_vars(branch)?;
                }
            }
            Statement::While { body, .. } => self.collect_vars(body)?,
            Statement::FunctionDeclaration { .. }
            | Statement::Expression(_)
            | Statement::Return(_)
            | Statement::VariableDeclaration { .. } => {}
        }
        Ok(())
    }

    fn add_binding(
        &mut self,
        scope: usize,
        name: &str,
        kind: VarKind,
    ) -> Result<Binding, CompileError> {
        if let Some(existing) = self.scopes[scope].bindings.get(name).copied() {
            if existing.kind == VarKind::Var && kind == VarKind::Var {
                return Ok(existing);
            }
            return Err(CompileError {
                message: format!("redeclaration of `{name}`"),
                unsupported: false,
            });
        }
        let slot = self.scopes[scope].slots;
        self.scopes[scope].slots = slot.checked_add(1).ok_or_else(|| CompileError {
            message: "scope has too many bindings".into(),
            unsupported: false,
        })?;
        let binding = Binding { scope, slot, kind };
        self.scopes[scope].bindings.insert(name.to_owned(), binding);
        Ok(binding)
    }

    fn emit_scope_declarations(&mut self, scope: usize) -> Result<(), CompileError> {
        let bindings: Vec<_> = self.scopes[scope]
            .bindings
            .iter()
            .map(|(name, binding)| (name.clone(), *binding))
            .collect();
        for (name, binding) in bindings {
            let name = self.name(&name)?;
            if scope == 0 {
                self.chunk.emit(Instruction::DeclareGlobal {
                    slot: binding.slot,
                    kind: binding.kind,
                    name,
                });
            } else {
                self.chunk.emit(Instruction::DeclareLocal {
                    slot: binding.slot,
                    kind: binding.kind,
                    name,
                });
            }
        }
        Ok(())
    }

    fn enter_block(&mut self) -> Result<usize, CompileError> {
        let scope = self.scopes.len();
        self.scopes.push(Scope {
            bindings: HashMap::new(),
            slots: 0,
        });
        Ok(scope)
    }

    fn collect_block_bindings(
        &mut self,
        scope: usize,
        statements: &[Statement],
    ) -> Result<(), CompileError> {
        for statement in statements {
            if let Statement::VariableDeclaration { kind, name, .. } = statement {
                if *kind != VarKind::Var {
                    self.add_binding(scope, name, *kind)?;
                }
            }
        }
        Ok(())
    }

    fn binding(&self, name: &str) -> Option<Binding> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.bindings.get(name).copied())
    }

    fn name(&mut self, name: &str) -> Result<u16, CompileError> {
        self.chunk.add_name(name).map_err(|message| CompileError {
            message: message.into(),
            unsupported: false,
        })
    }

    fn compile_sequence(
        &mut self,
        statements: &[Statement],
        wants_completion: bool,
    ) -> Result<bool, CompileError> {
        let mut has_value = false;
        for statement in statements {
            let produced = self.compile_statement(statement, true)?;
            if produced {
                if has_value {
                    self.chunk.emit(Instruction::Swap);
                    self.chunk.emit(Instruction::Pop);
                }
                has_value = true;
            }
        }
        if wants_completion {
            if !has_value {
                self.constant(Value::Undefined)?;
            }
            Ok(true)
        } else {
            if has_value {
                self.chunk.emit(Instruction::Pop);
            }
            Ok(false)
        }
    }

    fn compile_statement(
        &mut self,
        statement: &Statement,
        is_completion: bool,
    ) -> Result<bool, CompileError> {
        match statement {
            Statement::Expression(expression) => {
                self.compile_expression(expression)?;
                if !is_completion {
                    self.chunk.emit(Instruction::Pop);
                }
                Ok(is_completion)
            }
            Statement::Block(statements) => {
                let scope = self.enter_block()?;
                self.collect_block_bindings(scope, statements)?;
                let enter = self.chunk.emit(Instruction::EnterScope(0));
                self.emit_scope_declarations(scope)?;
                self.compile_sequence(statements, true)?;
                self.chunk.code[enter] = Instruction::EnterScope(self.scopes[scope].slots);
                self.chunk.emit(Instruction::ExitScope);
                self.scopes.pop();
                if !is_completion {
                    self.chunk.emit(Instruction::Pop);
                }
                Ok(is_completion)
            }
            Statement::VariableDeclaration { kind, name, init } => {
                let name_index = self.name(name)?;
                if let Some(init) = init {
                    self.compile_expression(init)?;
                } else {
                    self.constant(Value::Undefined)?;
                }
                let binding = self.binding(name).ok_or_else(|| CompileError {
                    message: format!("unresolved declaration `{name}`"),
                    unsupported: false,
                })?;
                if binding.scope == 0 {
                    self.chunk.emit(Instruction::InitializeGlobal {
                        slot: binding.slot,
                        name: name_index,
                    });
                } else {
                    self.chunk.emit(Instruction::InitializeLocal {
                        slot: binding.slot,
                        name: name_index,
                    });
                }
                let _ = kind;
                Ok(false)
            }
            Statement::If {
                condition,
                then_branch,
                else_branch,
            } => self.compile_if(condition, then_branch, else_branch, is_completion),
            Statement::While { condition, body } => {
                self.compile_while(condition, body, is_completion)
            }
            Statement::FunctionDeclaration { .. } => {
                Err(CompileError::unsupported("function declarations"))
            }
            Statement::Return(_) => Err(CompileError::unsupported("return statements")),
        }
    }

    fn compile_expression(&mut self, expression: &Expr) -> Result<(), CompileError> {
        match expression {
            Expr::Number(value) => self.constant(Value::Number(*value)),
            Expr::String(value) => self.constant(Value::String(value.clone())),
            Expr::Boolean(value) => self.constant(Value::Boolean(*value)),
            Expr::Null => self.constant(Value::Null),
            Expr::Undefined => self.constant(Value::Undefined),
            Expr::Unary { operator, operand } => {
                self.compile_expression(operand)?;
                self.chunk.emit(Instruction::Unary(*operator));
                Ok(())
            }
            Expr::Binary {
                left,
                operator,
                right,
            } => {
                self.compile_expression(left)?;
                self.compile_expression(right)?;
                self.chunk.emit(Instruction::Binary(*operator));
                Ok(())
            }
            Expr::Logical {
                left,
                operator,
                right,
            } => self.compile_logical(left, *operator, right),
            Expr::Identifier(name) => self.compile_load(name),
            Expr::Assign { name, value } => {
                self.compile_expression(value)?;
                self.compile_store(name)
            }
            Expr::Call { .. } => Err(CompileError::unsupported("function calls")),
            Expr::Member { object, property } => {
                self.compile_expression(object)?;
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
            Expr::MemberAssign {
                object,
                property,
                value,
            } => {
                self.compile_expression(object)?;
                match property {
                    MemberProperty::Named(name) => {
                        let index = self.name_constant(name)?;
                        self.compile_expression(value)?;
                        self.chunk.emit(Instruction::StoreProperty(index));
                    }
                    MemberProperty::Computed(key) => {
                        self.compile_expression(key)?;
                        self.compile_expression(value)?;
                        self.chunk.emit(Instruction::StoreComputedProperty);
                    }
                }
                Ok(())
            }
        }
    }

    /// Adds a property name to the constant pool and returns its index.
    fn name_constant(&mut self, name: &str) -> Result<u16, CompileError> {
        self.chunk
            .add_constant(Value::String(name.to_owned()))
            .map_err(|message| CompileError {
                message: message.into(),
                unsupported: false,
            })
    }

    fn compile_load(&mut self, name: &str) -> Result<(), CompileError> {
        let name_index = self.name(name)?;
        let Some(binding) = self.binding(name) else {
            self.chunk
                .emit(Instruction::LoadUnresolved { name: name_index });
            return Ok(());
        };
        if binding.scope == 0 {
            self.chunk.emit(Instruction::LoadGlobal {
                slot: binding.slot,
                name: name_index,
            });
        } else {
            let depth =
                u16::try_from(self.scopes.len() - 1 - binding.scope).map_err(|_| CompileError {
                    message: "scope nesting is too deep".into(),
                    unsupported: false,
                })?;
            self.chunk.emit(Instruction::LoadLocal {
                depth,
                slot: binding.slot,
                name: name_index,
            });
        }
        Ok(())
    }

    fn compile_store(&mut self, name: &str) -> Result<(), CompileError> {
        let name_index = self.name(name)?;
        let Some(binding) = self.binding(name) else {
            self.chunk
                .emit(Instruction::StoreUnresolved { name: name_index });
            return Ok(());
        };
        if binding.scope == 0 {
            self.chunk.emit(Instruction::StoreGlobal {
                slot: binding.slot,
                name: name_index,
            });
        } else {
            let depth =
                u16::try_from(self.scopes.len() - 1 - binding.scope).map_err(|_| CompileError {
                    message: "scope nesting is too deep".into(),
                    unsupported: false,
                })?;
            self.chunk.emit(Instruction::StoreLocal {
                depth,
                slot: binding.slot,
                name: name_index,
            });
        }
        Ok(())
    }

    fn compile_logical(
        &mut self,
        left: &Expr,
        operator: LogicalOperator,
        right: &Expr,
    ) -> Result<(), CompileError> {
        self.compile_expression(left)?;
        let jump = match operator {
            LogicalOperator::And => self.chunk.emit(Instruction::JumpIfFalse(usize::MAX)),
            LogicalOperator::Or => self.chunk.emit(Instruction::JumpIfTrue(usize::MAX)),
        };
        self.chunk.emit(Instruction::Pop);
        self.compile_expression(right)?;
        self.chunk.code[jump] = match operator {
            LogicalOperator::And => Instruction::JumpIfFalse(self.chunk.code.len()),
            LogicalOperator::Or => Instruction::JumpIfTrue(self.chunk.code.len()),
        };
        Ok(())
    }

    fn constant(&mut self, value: Value) -> Result<(), CompileError> {
        let index = self
            .chunk
            .add_constant(value)
            .map_err(|message| CompileError {
                message: message.into(),
                unsupported: false,
            })?;
        self.chunk.emit(Instruction::Constant(index));
        Ok(())
    }
}

impl Default for Compiler {
    fn default() -> Self {
        Self::new()
    }
}
