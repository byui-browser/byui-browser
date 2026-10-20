//! AST-to-bytecode compiler.

use super::{Chunk, Instruction};
use crate::Value;
use crate::ast::{Expr, LogicalOperator, MemberProperty, Program, Statement};
use std::fmt;

/// A source construct that the initial bytecode compiler cannot lower yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    /// Human-readable description of the unsupported construct.
    pub message: String,
}

impl CompileError {
    fn unsupported(construct: &str) -> Self {
        Self {
            message: format!("bytecode compiler does not support {construct} yet"),
        }
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CompileError {}

/// Compiles the expression subset into a fresh [`Chunk`].
#[derive(Debug, Default)]
pub struct Compiler {
    chunk: Chunk,
}

impl Compiler {
    /// Creates a compiler with an empty constant pool and instruction stream.
    pub fn new() -> Self {
        Self::default()
    }

    /// Compiles `program`, returning an owned chunk ready for [`crate::vm::Vm`].
    pub fn compile(mut self, program: &Program) -> Result<Chunk, CompileError> {
        for (index, statement) in program.body.iter().enumerate() {
            self.compile_statement(statement, index + 1 == program.body.len())?;
        }
        self.chunk.emit(Instruction::Return);
        Ok(self.chunk)
    }

    fn compile_statement(
        &mut self,
        statement: &Statement,
        is_completion: bool,
    ) -> Result<(), CompileError> {
        match statement {
            Statement::Expression(expression) => {
                self.compile_expression(expression)?;
                if !is_completion {
                    self.chunk.emit(Instruction::Pop);
                }
                Ok(())
            }
            Statement::Block(statements) => {
                for (index, statement) in statements.iter().enumerate() {
                    self.compile_statement(
                        statement,
                        is_completion && index + 1 == statements.len(),
                    )?;
                }
                Ok(())
            }
            Statement::VariableDeclaration { .. } => Err(CompileError::unsupported(
                "variable declarations and lexical environments",
            )),
            Statement::If { .. } => Err(CompileError::unsupported("if statements")),
            Statement::While { .. } => Err(CompileError::unsupported("while statements")),
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
            Expr::Identifier(_) | Expr::Assign { .. } => {
                Err(CompileError::unsupported("identifier bindings"))
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
            })
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
            })?;
        self.chunk.emit(Instruction::Constant(index));
        Ok(())
    }
}
