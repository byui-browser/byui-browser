use crate::ast::{Expr, LogicalOperator};
use crate::lexer::Token;

pub(super) fn from_token(token: &Token) -> Option<LogicalOperator> {
    match token {
        Token::AndAnd => Some(LogicalOperator::And),
        Token::OrOr => Some(LogicalOperator::Or),
        _ => None,
    }
}

impl LogicalOperator {
    /// Binding power; lower than every binary operator.
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
