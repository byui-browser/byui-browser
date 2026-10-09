mod assignment;
mod binary_operator;
mod call;
mod logical;
mod unary;

use super::{ParseResult, Parser};
use crate::ast::Expr;
use crate::lexer::Token;

/// Parses an expression, returning it with the depth of its tree.
pub(super) fn parse(parser: &mut Parser<'_>, minimum_precedence: u8) -> ParseResult<(Expr, usize)> {
    let (mut left, mut depth) = unary::parse(parser)?;
    loop {
        if minimum_precedence == 0 && parser.peek() == Some(&Token::Assign) {
            let Expr::Identifier(name) = left else {
                return Err(parser.error("Assignment target must be an identifier", "assignment"));
            };
            parser.advance();
            let (value, value_depth) = parser.parse_expression_with_depth(0)?;
            depth = parser.node_depth(value_depth + 1, "assignment")?;
            left = assignment::build(name, value);
        } else if let Some(operator) = parser.peek().and_then(binary_operator::from_token) {
            if operator.precedence() < minimum_precedence {
                break;
            }
            parser.advance();
            let (right, right_depth) =
                parser.parse_expression_with_depth(operator.precedence() + 1)?;
            depth = parser.node_depth(depth.max(right_depth) + 1, "expression")?;
            left = operator.build(left, right);
        } else if let Some(operator) = parser.peek().and_then(logical::from_token) {
            if operator.precedence() < minimum_precedence {
                break;
            }
            parser.advance();
            let (right, right_depth) =
                parser.parse_expression_with_depth(operator.precedence() + 1)?;
            depth = parser.node_depth(depth.max(right_depth) + 1, "expression")?;
            left = operator.build(left, right);
        } else {
            break;
        }
    }
    Ok((left, depth))
}
