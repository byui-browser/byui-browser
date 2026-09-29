use crate::ast::{BinaryOperator, Expr};
use crate::lexer::Token;

pub(super) fn from_token(token: &Token) -> Option<BinaryOperator> {
    match token {
        Token::Plus => Some(BinaryOperator::Add),
        Token::Subtract => Some(BinaryOperator::Subtract),
        Token::Multiply => Some(BinaryOperator::Multiply),
        Token::Divide => Some(BinaryOperator::Divide),
        Token::Number(_) => None,
    }
}

impl BinaryOperator {
    pub(super) fn precedence(self) -> u8 {
        match self {
            Self::Equal | Self::NotEqual | Self::StrictEqual | Self::StrictNotEqual => 1,
            Self::Less | Self::LessEqual | Self::Greater | Self::GreaterEqual => 2,
            Self::Add | Self::Subtract => 3,
            Self::Multiply | Self::Divide | Self::Remainder => 4,
        }
    }

    pub(super) fn build(self, left: Expr, right: Expr) -> Expr {
        Expr::Binary {
            left: Box::new(left),
            operator: self,
            right: Box::new(right),
        }
    }
}
