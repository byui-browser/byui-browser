mod assignment;
mod binary_operator;
mod call;
mod logical;
mod object;
mod unary;

pub(in crate::parser) use call::property_name;

use super::{ParseResult, Parser};
use crate::ast::{ArrowBody, Expr};
use crate::lexer::Token;

/// Parses an expression, returning it with the depth of its tree.
pub(super) fn parse(parser: &mut Parser<'_>, minimum_precedence: u8) -> ParseResult<(Expr, usize)> {
    if minimum_precedence == 0 && is_arrow_start(parser) {
        return parse_arrow(parser);
    }
    let (mut left, mut depth) = unary::parse(parser)?;
    loop {
        if minimum_precedence == 0 && parser.peek() == Some(&Token::Assign) {
            if !matches!(left, Expr::Identifier(_) | Expr::Member { .. }) {
                return Err(parser.error(
                    "Assignment target must be an identifier or member expression",
                    "assignment",
                ));
            }
            parser.advance();
            let (value, value_depth) = parser.parse_expression_with_depth(0)?;
            depth = parser.node_depth(depth.max(value_depth) + 1, "assignment")?;
            left = assignment::build(left, value);
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

fn is_arrow_start(parser: &Parser<'_>) -> bool {
    if matches!(parser.peek(), Some(Token::Identifier(_)))
        && parser.peek_at(1) == Some(&Token::Arrow)
    {
        return true;
    }
    if parser.peek() != Some(&Token::LeftParen) {
        return false;
    }
    let mut offset = 1;
    if parser.peek_at(offset) == Some(&Token::RightParen) {
        return parser.peek_at(offset + 1) == Some(&Token::Arrow);
    }
    loop {
        if !matches!(parser.peek_at(offset), Some(Token::Identifier(_))) {
            return false;
        }
        offset += 1;
        match parser.peek_at(offset) {
            Some(Token::Comma) => offset += 1,
            Some(Token::RightParen) => return parser.peek_at(offset + 1) == Some(&Token::Arrow),
            _ => return false,
        }
    }
}

fn parse_arrow(parser: &mut Parser<'_>) -> ParseResult<(Expr, usize)> {
    let mut params = Vec::new();
    if parser.consume(&Token::LeftParen) {
        if !parser.consume(&Token::RightParen) {
            loop {
                let name = parser.identifier("arrow parameters")?;
                if params.contains(&name) {
                    return Err(parser.error("Duplicate parameter name", "arrow parameters"));
                }
                params.push(name);
                if !parser.consume(&Token::Comma) {
                    break;
                }
            }
            parser.expect(&Token::RightParen, "arrow parameters")?;
        }
    } else {
        params.push(parser.identifier("arrow parameter")?);
    }
    parser.expect(&Token::Arrow, "arrow function")?;
    parser.function_depth += 1;
    let parsed_body = (|| {
        if parser.peek() == Some(&Token::LeftBrace) {
            Ok(ArrowBody::Block(
                crate::parser::statements::parse_arrow_block(parser)?,
            ))
        } else {
            let (expression, _) = parser.parse_expression_with_depth(0)?;
            Ok(ArrowBody::Expression(Box::new(expression)))
        }
    })();
    parser.function_depth -= 1;
    Ok((
        Expr::ArrowFunction {
            params,
            body: parsed_body?,
        },
        1,
    ))
}
