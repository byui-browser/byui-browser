// TODO(parser): Add `return` token parsing once the lexer exposes the keyword
// and statement terminators.

use crate::ast::{Expr, Statement};

impl Statement {
    pub(super) fn return_statement(value: Option<Expr>) -> Self {
        Self::Return(value)
    }

    pub(super) fn expression(expression: Expr) -> Self {
        Self::Expression(expression)
    }
}
