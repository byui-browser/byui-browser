use crate::ast::{Expr, LogicalOperator};
use crate::lexer::Token;

pub(super) fn from_token(token: &Token) -> Option<LogicalOperator> {
    match token {
        Token::AndAnd => Some(LogicalOperator::And),
        Token::OrOr => Some(LogicalOperator::Or),
        Token::NullishCoalesce => Some(LogicalOperator::Nullish),
        _ => None,
    }
}

impl LogicalOperator {
    /// Binding power; lower than every binary operator.
    pub(super) fn precedence(self) -> u8 {
        match self {
            Self::Or | Self::Nullish => 1,
            Self::And => 2,
        }
    }

    /// Minimum binding power of the right operand.
    ///
    /// `??` takes a right operand that binds tighter than `&&`, so
    /// `a ?? b && c` leaves the `&&` for the caller to reject: JavaScript
    /// forbids mixing `??` with `||` or `&&` without parentheses.
    pub(super) fn right_precedence(self) -> u8 {
        match self {
            Self::Nullish => 3,
            _ => self.precedence() + 1,
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
