//! Lowering for structured control-flow statements.

use super::Control;
use super::{CompileError, Compiler, Instruction};
use crate::ast::{CatchClause, Expr, Statement, VarKind};
use std::rc::Rc;

impl Compiler {
    pub(super) fn patch(&mut self, jump: usize) -> Result<(), super::CompileError> {
        self.chunk
            .patch_jump(jump, self.chunk.current_offset())
            .map_err(|message| super::CompileError {
                message: message.into(),
                unsupported: false,
                location: None,
            })
    }

    pub(super) fn emit_placeholder(
        &mut self,
        instruction: Instruction,
    ) -> Result<usize, super::CompileError> {
        self.chunk
            .emit_jump(instruction)
            .map_err(|message| super::CompileError {
                message: message.into(),
                unsupported: false,
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
        self.live += 1;
        let loop_start = self.chunk.current_offset();
        self.control.push(Control::Loop {
            live: self.live,
            scopes: self.scopes.len(),
            continue_scopes: self.scopes.len(),
            continue_target: loop_start,
            breaks: Vec::new(),
            continues: Vec::new(),
        });
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
        // `break` arrives here with only the loop's completion value pushed.
        let Some(Control::Loop { breaks, .. }) = self.control.pop() else {
            return Err(internal("loop control entry is missing"));
        };
        for jump in breaks {
            self.patch(jump)?;
        }
        self.live -= 1;
        if !is_completion {
            self.chunk.emit(Instruction::Pop);
        }
        Ok(is_completion)
    }

    pub(super) fn compile_for(
        &mut self,
        init: Option<&Statement>,
        condition: Option<&Expr>,
        update: Option<&Expr>,
        body: &Statement,
        is_completion: bool,
    ) -> Result<bool, super::CompileError> {
        let outer_scope = self.scopes.len();
        let loop_scope = self.enter_block()?;
        if let Some(Statement::VariableDeclaration { kind, name, .. }) = init
            && *kind != VarKind::Var
        {
            self.add_binding(loop_scope, name, *kind)?;
        }
        let enter = self.chunk.emit(Instruction::EnterScope(0));
        self.emit_scope_declarations(loop_scope)?;
        if let Some(init) = init {
            self.compile_statement(init, false)?;
        }
        self.constant(crate::Value::Undefined)?;
        self.live += 1;
        let loop_start = self.chunk.current_offset();
        self.control.push(Control::Loop {
            live: self.live,
            scopes: outer_scope,
            continue_scopes: loop_scope + 1,
            continue_target: usize::MAX,
            breaks: Vec::new(),
            continues: Vec::new(),
        });
        let exit = if let Some(condition) = condition {
            self.compile_expression(condition)?;
            let exit = self.emit_placeholder(Instruction::JumpIfFalse(usize::MAX))?;
            self.chunk.emit(Instruction::Pop);
            Some(exit)
        } else {
            None
        };
        let body_produced = self.compile_statement(body, true)?;
        if body_produced {
            self.chunk.emit(Instruction::Swap);
            self.chunk.emit(Instruction::Pop);
        }
        let update_target = self.chunk.current_offset();
        let continues = match self.control.last_mut() {
            Some(Control::Loop {
                continue_target,
                continues,
                ..
            }) => {
                *continue_target = update_target;
                std::mem::take(continues)
            }
            _ => return Err(internal("loop control entry is missing")),
        };
        for jump in continues {
            self.patch_to(jump, update_target)?;
        }
        if let Some(update) = update {
            self.compile_expression(update)?;
            self.chunk.emit(Instruction::Pop);
        }
        self.chunk.emit(Instruction::Jump(loop_start));
        if let Some(exit) = exit {
            self.patch(exit)?;
            self.chunk.emit(Instruction::Pop);
        }
        self.chunk.emit(Instruction::ExitScope);
        let Some(Control::Loop { breaks, .. }) = self.control.pop() else {
            return Err(internal("loop control entry is missing"));
        };
        for jump in breaks {
            self.patch(jump)?;
        }
        self.chunk.code[enter] = Instruction::EnterScope(self.scopes[loop_scope].slots);
        self.scopes.pop();
        self.live -= 1;
        if !is_completion {
            self.chunk.emit(Instruction::Pop);
        }
        Ok(is_completion)
    }

    pub(super) fn patch_to(
        &mut self,
        jump: usize,
        target: usize,
    ) -> Result<(), super::CompileError> {
        self.chunk
            .patch_jump(jump, target)
            .map_err(|message| super::CompileError {
                message: message.into(),
                unsupported: false,
                location: None,
            })
    }
}

fn internal(message: &str) -> CompileError {
    CompileError {
        message: message.into(),
        unsupported: false,
        location: None,
    }
}

impl Compiler {
    /// Lowers `break` or `continue`: leaves every handler between here and
    /// the loop, running `finally` blocks, discards the values and scopes
    /// opened inside the loop body, then jumps.
    pub(super) fn compile_loop_jump(
        &mut self,
        is_break: bool,
        is_completion: bool,
    ) -> Result<bool, CompileError> {
        let index = self
            .control
            .iter()
            .rposition(|entry| matches!(entry, Control::Loop { .. }))
            .ok_or_else(|| {
                internal(if is_break {
                    "`break` outside a loop"
                } else {
                    "`continue` outside a loop"
                })
            })?;
        let Control::Loop {
            live,
            scopes,
            continue_scopes,
            continue_target,
            ..
        } = self.control[index]
        else {
            return Err(internal("loop control entry is missing"));
        };
        let end_scopes = if is_break { scopes } else { continue_scopes };
        self.unwind(index + 1, Some(live), Some(end_scopes))?;
        if is_break {
            let jump = self.emit_placeholder(Instruction::Jump(usize::MAX))?;
            if let Control::Loop { breaks, .. } = &mut self.control[index] {
                breaks.push(jump);
            }
        } else {
            if continue_target == usize::MAX {
                let jump = self.emit_placeholder(Instruction::Jump(usize::MAX))?;
                if let Control::Loop { continues, .. } = &mut self.control[index] {
                    continues.push(jump);
                }
            } else {
                self.chunk.emit(Instruction::Jump(continue_target));
            }
        }
        // Unreachable, but keeps the stack shape that callers expect.
        if is_completion {
            self.constant(crate::Value::Undefined)?;
        }
        Ok(is_completion)
    }

    /// Emits the code for leaving the handler regions in `control[floor..]`,
    /// innermost first: restore the scope depth the handler was entered at,
    /// deregister it, and run its `finally` block inline.
    ///
    /// `discard_to` is the number of statement values to keep: `break` and
    /// `continue` pop the rest, while `return` keeps its pending value and
    /// everything under it. `end_scopes` is the scope count to leave behind
    /// afterwards, or `None` when the frame is about to be left anyway.
    pub(super) fn unwind(
        &mut self,
        floor: usize,
        discard_to: Option<usize>,
        end_scopes: Option<usize>,
    ) -> Result<(), CompileError> {
        let saved_live = self.live;
        let mut scopes_now = self.scopes.len();
        let mut index = self.control.len();
        while index > floor {
            index -= 1;
            let Control::Handler {
                live,
                scopes,
                finally,
            } = self.control[index].clone()
            else {
                continue;
            };
            if discard_to.is_some() {
                self.emit_pops(self.live - live);
                self.live = live;
            }
            self.emit_exit_scopes(scopes_now - scopes);
            scopes_now = scopes;
            self.chunk.emit(Instruction::ExitTry);
            if let Some(finally) = finally {
                // The block runs outside the region it finishes: compile it
                // with the scopes and handlers that were visible there.
                let outer_control = self.control.split_off(index);
                let inner_scopes = self.scopes.split_off(scopes);
                let compiled = self.compile_block(&finally, false);
                self.scopes.extend(inner_scopes);
                self.control.extend(outer_control);
                compiled?;
            }
        }
        if let Some(live) = discard_to {
            self.emit_pops(self.live - live);
        }
        if let Some(end_scopes) = end_scopes {
            self.emit_exit_scopes(scopes_now - end_scopes);
        }
        self.live = saved_live;
        Ok(())
    }

    fn emit_pops(&mut self, count: usize) {
        for _ in 0..count {
            self.chunk.emit(Instruction::Pop);
        }
    }

    fn emit_exit_scopes(&mut self, count: usize) {
        for _ in 0..count {
            self.chunk.emit(Instruction::ExitScope);
        }
    }

    /// Lowers `try`/`catch`/`finally`. The statement always leaves one
    /// completion value: the `try` block's, or the `catch` block's.
    ///
    /// A `finally` handler wraps the `try` and `catch` blocks, and the
    /// `finally` block is compiled again at the end of the normal path, at
    /// every `return`, `break`, or `continue` leaving the statement, and once
    /// for exceptions, where it ends by rethrowing.
    pub(super) fn compile_try(
        &mut self,
        block: &[Statement],
        handler: Option<&CatchClause>,
        finalizer: Option<&[Statement]>,
        is_completion: bool,
    ) -> Result<bool, CompileError> {
        let base = self.live;
        let scopes = self.scopes.len();
        let finalizer: Option<Rc<[Statement]>> = finalizer.map(Rc::from);
        let finally_enter = if finalizer.is_some() {
            let enter = self.emit_placeholder(Instruction::EnterTry(usize::MAX))?;
            self.control.push(Control::Handler {
                live: base,
                scopes,
                finally: finalizer.clone(),
            });
            Some(enter)
        } else {
            None
        };
        if let Some(clause) = handler {
            let catch_enter = self.emit_placeholder(Instruction::EnterTry(usize::MAX))?;
            self.control.push(Control::Handler {
                live: base,
                scopes,
                finally: None,
            });
            self.compile_block(block, true)?;
            self.control.pop();
            self.chunk.emit(Instruction::ExitTry);
            let skip_catch = self.emit_placeholder(Instruction::Jump(usize::MAX))?;
            // The VM resumes here with the caught value pushed.
            self.patch(catch_enter)?;
            self.compile_catch(clause)?;
            self.patch(skip_catch)?;
        } else {
            self.compile_block(block, true)?;
        }
        if let (Some(enter), Some(finalizer)) = (finally_enter, finalizer) {
            self.control.pop();
            self.chunk.emit(Instruction::ExitTry);
            // The completion value waits under the `finally` block.
            self.live = base + 1;
            self.compile_block(&finalizer, false)?;
            let end = self.emit_placeholder(Instruction::Jump(usize::MAX))?;
            // Exception path: the thrown value is on the stack.
            self.patch(enter)?;
            self.compile_block(&finalizer, false)?;
            self.chunk.emit(Instruction::Throw);
            self.patch(end)?;
        }
        self.live = base;
        if !is_completion {
            self.chunk.emit(Instruction::Pop);
        }
        Ok(is_completion)
    }

    /// Lowers a `catch` clause entered with the caught value on the stack.
    fn compile_catch(&mut self, clause: &CatchClause) -> Result<(), CompileError> {
        match &clause.param {
            Some(param) => {
                let scope = self.enter_block()?;
                self.add_binding(scope, param, VarKind::Let)?;
                let enter = self.chunk.emit(Instruction::EnterScope(0));
                self.emit_scope_declarations(scope)?;
                self.emit_initialize(param)?;
                self.compile_block(&clause.body, true)?;
                self.chunk.code[enter] = Instruction::EnterScope(self.scopes[scope].slots);
                self.chunk.emit(Instruction::ExitScope);
                self.scopes.pop();
            }
            None => {
                self.chunk.emit(Instruction::Pop);
                self.compile_block(&clause.body, true)?;
            }
        }
        Ok(())
    }
}
