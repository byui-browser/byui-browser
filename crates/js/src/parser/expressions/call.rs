use super::super::{ParseResult, Parser};
use crate::ast::Expr;
use crate::lexer::Token;

pub(super) fn parse(parser: &mut Parser<'_>) -> ParseResult<(Expr, usize)> {
    let expression = match parser.peek() {
        Some(Token::Number(value)) => Expr::Number(*value),
        Some(Token::String(value)) => Expr::String(value.clone()),
        Some(Token::Identifier(name)) => Expr::Identifier(name.clone()),
        Some(Token::True) => Expr::Boolean(true),
        Some(Token::False) => Expr::Boolean(false),
        Some(Token::Null) => Expr::Null,
        Some(Token::Undefined) => Expr::Undefined,
        Some(Token::LeftParen) => {
            parser.advance();
            let (expression, depth) = parser.parse_expression_with_depth(0)?;
            parser.expect(&Token::RightParen, "parenthesized expression")?;
            return suffix(parser, expression, depth);
        }
        _ => return Err(parser.error("Expected an expression", "expression")),
    };
    parser.advance();
    suffix(parser, expression, 1)
}

fn suffix(
    parser: &mut Parser<'_>,
    mut callee: Expr,
    mut depth: usize,
) -> ParseResult<(Expr, usize)> {
    while parser.consume(&Token::LeftParen) {
        let mut arguments = Vec::new();
        let mut arguments_depth = 0;
        if !parser.consume(&Token::RightParen) {
            loop {
                let (argument, argument_depth) = parser.parse_expression_with_depth(0)?;
                arguments.push(argument);
                arguments_depth = arguments_depth.max(argument_depth);
                if !parser.consume(&Token::Comma) {
                    break;
                }
            }
            parser.expect(&Token::RightParen, "call arguments")?;
        }
        depth = parser.node_depth(depth.max(arguments_depth) + 1, "call arguments")?;
        callee = Expr::Call {
            callee: Box::new(callee),
            arguments,
        };
    }
    Ok((callee, depth))
}
