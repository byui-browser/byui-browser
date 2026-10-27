//! Lowering for structured control-flow statements.

use super::{Compiler, Instruction};
use crate::ast::{Expr, Statement};

impl Compiler {
    fn patch(&mut self, jump: usize) -> Result<(), super::CompileError> {
        self.chunk
            .patch_jump(jump, self.chunk.current_offset())
            .map_err(|message| super::CompileError {
                message: message.into(),
                location: None,
            })
    }

    fn emit_placeholder(&mut self, instruction: Instruction) -> Result<usize, super::CompileError> {
        self.chunk
            .emit_jump(instruction)
            .map_err(|message| super::CompileError {
                message: message.into(),
                location: None,
            })
    }

    fn ensure_value(&mut self, produced: bool) -> Result<(), super::CompileError> {
        if !produced {
            self.constant(crate::Value::Undefined)?;
        }
        Ok(())
    }

    pub(super) fn compile_if(
        &mut self,
        condition: &Expr,
        then_branch: &Statement,
        else_branch: &Option<Box<Statement>>,
        is_completion: bool,
    ) -> Result<bool, super::CompileError> {
        self.compile_expression(condition)?;
        let otherwise = self.emit_placeholder(Instruction::JumpIfFalse(usize::MAX))?;
        self.chunk.emit(Instruction::Pop);
        let then_produced = self.compile_statement(then_branch, true)?;
        self.ensure_value(then_produced)?;
        let end = self.emit_placeholder(Instruction::Jump(usize::MAX))?;
        self.patch(otherwise)?;
        self.chunk.emit(Instruction::Pop);
        if let Some(else_branch) = else_branch {
            let else_produced = self.compile_statement(else_branch, true)?;
            self.ensure_value(else_produced)?;
        } else {
            self.constant(crate::Value::Undefined)?;
        }
        self.patch(end)?;
        if !is_completion {
            self.chunk.emit(Instruction::Pop);
        }
        Ok(is_completion)
    }

    pub(super) fn compile_while(
        &mut self,
        condition: &Expr,
        body: &Statement,
        is_completion: bool,
    ) -> Result<bool, super::CompileError> {
        self.constant(crate::Value::Undefined)?;
        let loop_start = self.chunk.current_offset();
        self.compile_expression(condition)?;
        let exit = self.emit_placeholder(Instruction::JumpIfFalse(usize::MAX))?;
        self.chunk.emit(Instruction::Pop);
        let body_produced = self.compile_statement(body, true)?;
        if body_produced {
            self.chunk.emit(Instruction::Swap);
            self.chunk.emit(Instruction::Pop);
        }
        self.chunk.emit(Instruction::Jump(loop_start));
        self.patch(exit)?;
        self.chunk.emit(Instruction::Pop);
        if !is_completion {
            self.chunk.emit(Instruction::Pop);
        }
        Ok(is_completion)
    }
}
