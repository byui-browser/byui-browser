// TODO(parser): Add token-to-unary-operator parsing once the lexer exposes
// unary operator tokens such as `!`.

use crate::ast::{Expr, UnaryOperator};

impl UnaryOperator {
    pub(super) fn build(self, operand: Expr) -> Expr {
        Expr::Unary {
            operator: self,
            operand: Box::new(operand),
        }
    }
}
