// TODO(parser): Add `if`, `else`, and `while` token parsing once the lexer
// exposes the corresponding keywords and delimiters.

use crate::ast::{Expr, Statement};

impl Statement {
    pub(super) fn if_statement(
        condition: Expr,
        then_branch: Self,
        else_branch: Option<Self>,
    ) -> Self {
        Self::If {
            condition,
            then_branch: Box::new(then_branch),
            else_branch: else_branch.map(Box::new),
        }
    }

    pub(super) fn while_statement(condition: Expr, body: Self) -> Self {
        Self::While {
            condition,
            body: Box::new(body),
        }
    }
}
