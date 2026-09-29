// TODO(parser): Add `from_token` parsing once the lexer exposes `&&` and `||`.

use crate::ast::{Expr, LogicalOperator};

impl LogicalOperator {
    pub(super) fn precedence(self) -> u8 {
        match self {
            Self::Or => 1,
            Self::And => 2,
        }
    }

    pub(super) fn build(self, left: Expr, right: Expr) -> Expr {
        Expr::Logical {
            left: Box::new(left),
            operator: self,
            right: Box::new(right),
        }
    }
}
