use crate::ast::{BinaryOperator, Expr};
use crate::lexer::Token;

pub(super) fn from_token(token: &Token) -> Option<BinaryOperator> {
    match token {
        Token::Plus => Some(BinaryOperator::Add),
        Token::Subtract => Some(BinaryOperator::Subtract),
        Token::Multiply => Some(BinaryOperator::Multiply),
        Token::Divide => Some(BinaryOperator::Divide),
        Token::Remainder => Some(BinaryOperator::Remainder),
        Token::StarStar => Some(BinaryOperator::Exponent),
        Token::BitAnd => Some(BinaryOperator::BitwiseAnd),
        Token::BitOr => Some(BinaryOperator::BitwiseOr),
        Token::BitXor => Some(BinaryOperator::BitwiseXor),
        Token::ShiftLeft => Some(BinaryOperator::LeftShift),
        Token::ShiftRight => Some(BinaryOperator::SignedRightShift),
        Token::UnsignedShiftRight => Some(BinaryOperator::UnsignedRightShift),
        Token::LessEqual => Some(BinaryOperator::LessEqual),
        Token::GreaterEqual => Some(BinaryOperator::GreaterEqual),
        Token::BangEqual => Some(BinaryOperator::NotEqual),
        Token::StrictEqual => Some(BinaryOperator::StrictEqual),
        Token::StrictBangEqual => Some(BinaryOperator::StrictNotEqual),
        Token::EqualEqual => Some(BinaryOperator::Equal),
        Token::LessThan => Some(BinaryOperator::Less),
        Token::GreaterThan => Some(BinaryOperator::Greater),
        Token::In => Some(BinaryOperator::In),
        Token::InstanceOf => Some(BinaryOperator::InstanceOf),
        _ => None,
    }
}

impl BinaryOperator {
    /// Binding power; logical operators use 1 (`||`, `??`) and 2 (`&&`).
    pub(super) fn precedence(self) -> u8 {
        match self {
            Self::BitwiseOr => 3,
            Self::BitwiseXor => 4,
            Self::BitwiseAnd => 5,
            Self::Equal | Self::NotEqual | Self::StrictEqual | Self::StrictNotEqual => 6,
            Self::Less
            | Self::LessEqual
            | Self::Greater
            | Self::GreaterEqual
            | Self::In
            | Self::InstanceOf => 7,
            Self::LeftShift | Self::SignedRightShift | Self::UnsignedRightShift => 8,
            Self::Add | Self::Subtract => 9,
            Self::Multiply | Self::Divide | Self::Remainder => 10,
            Self::Exponent => 11,
        }
    }

    /// Minimum binding power of the right operand: one more than
    /// [`Self::precedence`] for left-associative operators, and equal to it
    /// for the right-associative `**`.
    pub(super) fn right_precedence(self) -> u8 {
        match self {
            Self::Exponent => self.precedence(),
            _ => self.precedence() + 1,
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
