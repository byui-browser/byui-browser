use crate::ast::{AssignmentOperator, BinaryOperator, Expr, LogicalOperator};
use crate::lexer::Token;

/// Builds an assignment to `target`, which the caller has checked is an
/// identifier or member expression.
pub(super) fn build(target: Expr, value: Expr) -> Expr {
    let value = Box::new(value);
    match target {
        Expr::Member { object, property } => Expr::MemberAssign {
            object,
            property,
            value,
        },
        Expr::Identifier(name) => Expr::Assign { name, value },
        _ => unreachable!("assignment target is validated by the caller"),
    }
}

/// Maps a compound assignment token such as `+=` or `??=` to its operator.
pub(super) fn compound_operator(token: &Token) -> Option<AssignmentOperator> {
    use AssignmentOperator::{Binary, Logical};
    Some(match token {
        Token::PlusAssign => Binary(BinaryOperator::Add),
        Token::MinusAssign => Binary(BinaryOperator::Subtract),
        Token::StarAssign => Binary(BinaryOperator::Multiply),
        Token::SlashAssign => Binary(BinaryOperator::Divide),
        Token::PercentAssign => Binary(BinaryOperator::Remainder),
        Token::StarStarAssign => Binary(BinaryOperator::Exponent),
        Token::ShiftLeftAssign => Binary(BinaryOperator::LeftShift),
        Token::ShiftRightAssign => Binary(BinaryOperator::SignedRightShift),
        Token::UnsignedShiftRightAssign => Binary(BinaryOperator::UnsignedRightShift),
        Token::BitAndAssign => Binary(BinaryOperator::BitwiseAnd),
        Token::BitOrAssign => Binary(BinaryOperator::BitwiseOr),
        Token::BitXorAssign => Binary(BinaryOperator::BitwiseXor),
        Token::AndAssign => Logical(LogicalOperator::And),
        Token::OrAssign => Logical(LogicalOperator::Or),
        Token::NullishAssign => Logical(LogicalOperator::Nullish),
        _ => return None,
    })
}

/// Builds a compound assignment to `target`, which the caller has checked is
/// an identifier or member expression.
pub(super) fn build_compound(target: Expr, operator: AssignmentOperator, value: Expr) -> Expr {
    Expr::CompoundAssign {
        target: Box::new(target),
        operator,
        value: Box::new(value),
    }
}
