use super::super::{ParseResult, Parser};
use crate::ast::Expr;
use crate::lexer::Token;

pub(super) fn parse(parser: &mut Parser<'_>) -> ParseResult<Expr> {
    let mut expression = match parser.peek() {
        Some(Token::Number(value)) => Expr::Number(*value),
        Some(Token::String(value)) => Expr::String(value.clone()),
        Some(Token::Identifier(name)) => Expr::Identifier(name.clone()),
        Some(Token::True) => Expr::Boolean(true),
        Some(Token::False) => Expr::Boolean(false),
        Some(Token::Null) => Expr::Null,
        Some(Token::Undefined) => Expr::Undefined,
        Some(Token::LeftParen) => {
            parser.advance();
            let expression = parser.parse_expression(0)?;
            parser.expect(&Token::RightParen, "parenthesized expression")?;
            return suffix(parser, expression);
        }
        _ => return Err(parser.error("Expected an expression", "expression")),
    };
    parser.advance();
    expression = suffix(parser, expression)?;
    Ok(expression)
}

fn suffix(parser: &mut Parser<'_>, mut callee: Expr) -> ParseResult<Expr> {
    while parser.consume(&Token::LeftParen) {
        let mut arguments = Vec::new();
        if !parser.consume(&Token::RightParen) {
            loop {
                arguments.push(parser.parse_expression(0)?);
                if !parser.consume(&Token::Comma) {
                    break;
                }
            }
            parser.expect(&Token::RightParen, "call arguments")?;
        }
        callee = Expr::Call {
            callee: Box::new(callee),
            arguments,
        };
    }
    Ok(callee)
}
