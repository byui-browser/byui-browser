use super::super::{ParseResult, Parser};
use crate::ast::{Expr, UnaryOperator};
use crate::lexer::Token;

pub(super) fn parse(parser: &mut Parser<'_>) -> ParseResult<Expr> {
    let operator = match parser.peek() {
        Some(Token::Subtract) => Some(UnaryOperator::Negate),
        Some(Token::Bang) => Some(UnaryOperator::Not),
        _ => None,
    };
    if let Some(operator) = operator {
        parser.advance();
        return parser.nested("unary expression", |parser| {
            Ok(Expr::Unary {
                operator,
                operand: Box::new(parse(parser)?),
            })
        });
    }
    super::call::parse(parser)
}
