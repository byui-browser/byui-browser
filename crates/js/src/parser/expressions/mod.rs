#[allow(dead_code)]
mod assignment;
mod binary_operator;
#[allow(dead_code)]
mod call;
#[allow(dead_code)]
mod logical;
#[allow(dead_code)]
mod unary;

use crate::ast::Expr;

use super::Parser;

pub(super) fn parse(parser: &mut Parser<'_>, minimum_precedence: u8) -> Result<Expr, String> {
    let mut left = parser.parse_unary()?;

    while let Some(operator) = parser.peek().and_then(binary_operator::from_token) {
        if operator.precedence() < minimum_precedence {
            break;
        }

        parser.advance();
        let right = parse(parser, operator.precedence() + 1)?;
        left = operator.build(left, right);
    }

    Ok(left)
}
