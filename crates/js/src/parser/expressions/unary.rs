use super::super::{ParseResult, Parser};
use crate::ast::{Expr, UnaryOperator};
use crate::lexer::Token;

pub(super) fn parse(parser: &mut Parser<'_>) -> ParseResult<(Expr, usize)> {
    let operator = match parser.peek() {
        Some(Token::Subtract) => Some(UnaryOperator::Negate),
        Some(Token::Bang) => Some(UnaryOperator::Not),
        Some(Token::BitNot) => Some(UnaryOperator::BitwiseNot),
        _ => None,
    };
    if let Some(operator) = operator {
        parser.advance();
        return parser.nested("unary expression", |parser| {
            let (operand, depth) = parse(parser)?;
            let depth = parser.node_depth(depth + 1, "unary expression")?;
            Ok((
                Expr::Unary {
                    operator,
                    operand: Box::new(operand),
                },
                depth,
            ))
        });
    }
    super::call::parse(parser)
}
