use crate::ast::{BinaryOperator, Expr};
use crate::lexer::Token;

pub(super) fn from_token(token: &Token) -> Option<BinaryOperator> {
    match token {
        Token::Plus => Some(BinaryOperator::Add),
        Token::Subtract => Some(BinaryOperator::Subtract),
        Token::Multiply => Some(BinaryOperator::Multiply),
        Token::Divide => Some(BinaryOperator::Divide),
        Token::LessEqual => Some(BinaryOperator::LessEqual),
        Token::GreaterEqual => Some(BinaryOperator::GreaterEqual),
        Token::BangEqual => Some(BinaryOperator::NotEqual),
        Token::StrictEqual => Some(BinaryOperator::StrictEqual),
        Token::StrictBangEqual => Some(BinaryOperator::StrictNotEqual),
        Token::EqualEqual => Some(BinaryOperator::Equal),
        Token::LessThan => Some(BinaryOperator::Less),
        Token::GreaterThan => Some(BinaryOperator::Greater),
        _ => None,
    }
}

impl BinaryOperator {
    pub(super) fn precedence(self) -> u8 {
        match self {
            Self::Equal | Self::NotEqual | Self::StrictEqual | Self::StrictNotEqual => 3,
            Self::Less | Self::LessEqual | Self::Greater | Self::GreaterEqual => 4,
            Self::Add | Self::Subtract => 5,
            Self::Multiply | Self::Divide | Self::Remainder => 6,
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
