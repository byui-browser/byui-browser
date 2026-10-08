use super::super::{ParseResult, Parser};
use crate::ast::{Expr, MemberProperty};
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

/// Parses any chain of call, `.name`, and `[key]` suffixes, left to right.
fn suffix(
    parser: &mut Parser<'_>,
    mut callee: Expr,
    mut depth: usize,
) -> ParseResult<(Expr, usize)> {
    loop {
        if parser.consume(&Token::LeftParen) {
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
        } else if parser.consume(&Token::Dot) {
            let name = property_name(parser)?;
            depth = parser.node_depth(depth + 1, "member access")?;
            callee = Expr::Member {
                object: Box::new(callee),
                property: MemberProperty::Named(name),
            };
        } else if parser.consume(&Token::LeftBracket) {
            let (key, key_depth) = parser.parse_expression_with_depth(0)?;
            parser.expect(&Token::RightBracket, "computed member access")?;
            depth = parser.node_depth(depth.max(key_depth) + 1, "computed member access")?;
            callee = Expr::Member {
                object: Box::new(callee),
                property: MemberProperty::Computed(Box::new(key)),
            };
        } else {
            return Ok((callee, depth));
        }
    }
}

/// Parses the name after `.`. Like JavaScript's `IdentifierName`, keywords
/// are allowed, so `object.if` and `object.null` are ordinary properties.
fn property_name(parser: &mut Parser<'_>) -> ParseResult<String> {
    let name = match parser.peek() {
        Some(Token::Identifier(name)) => name.as_str(),
        Some(Token::Let) => "let",
        Some(Token::Var) => "var",
        Some(Token::Const) => "const",
        Some(Token::If) => "if",
        Some(Token::Else) => "else",
        Some(Token::While) => "while",
        Some(Token::Function) => "function",
        Some(Token::Return) => "return",
        Some(Token::True) => "true",
        Some(Token::False) => "false",
        Some(Token::Null) => "null",
        Some(Token::Undefined) => "undefined",
        _ => return Err(parser.error("Expected a property name", "member access")),
    }
    .to_owned();
    parser.advance();
    Ok(name)
}
