mod assignment;
mod binary_operator;
mod call;
mod logical;
mod unary;

use super::{ParseResult, Parser};
use crate::ast::Expr;
use crate::lexer::Token;

pub(super) fn parse(parser: &mut Parser<'_>, minimum_precedence: u8) -> ParseResult<Expr> {
    let mut left = unary::parse(parser)?;
    loop {
        if minimum_precedence == 0 && parser.peek() == Some(&Token::Assign) {
            let Expr::Identifier(name) = left else {
                return Err(parser.error("Assignment target must be an identifier", "assignment"));
            };
            parser.advance();
            left = assignment::build(name, parser.parse_expression(0)?);
        } else if let Some(operator) = parser.peek().and_then(binary_operator::from_token) {
            if operator.precedence() < minimum_precedence {
                break;
            }
            parser.advance();
            let right = parser.parse_expression(operator.precedence() + 1)?;
            left = operator.build(left, right);
        } else if let Some(operator) = parser.peek().and_then(logical::from_token) {
            if operator.precedence() < minimum_precedence {
                break;
            }
            parser.advance();
            let right = parser.parse_expression(operator.precedence() + 1)?;
            left = operator.build(left, right);
        } else {
            break;
        }
    }
    Ok(left)
}
