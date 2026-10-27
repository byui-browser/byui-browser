use super::super::{ParseResult, Parser};
use crate::ast::{Expr, MemberProperty};
use crate::lexer::Token;

pub(super) fn parse(parser: &mut Parser<'_>) -> ParseResult<(Expr, usize)> {
    let (expression, depth) = primary(parser)?;
    suffix(parser, expression, depth, true)
}

/// Parses an operand without its call and member suffixes.
fn primary(parser: &mut Parser<'_>) -> ParseResult<(Expr, usize)> {
    let expression = match parser.peek() {
        Some(Token::Number(value)) => Expr::Number(*value),
        Some(Token::String(value)) => Expr::String(value.clone()),
        Some(Token::Identifier(name)) => Expr::Identifier(name.clone()),
        Some(Token::True) => Expr::Boolean(true),
        Some(Token::False) => Expr::Boolean(false),
        Some(Token::Null) => Expr::Null,
        Some(Token::Undefined) => Expr::Undefined,
        Some(Token::This) => Expr::This,
        Some(Token::New) => return parse_new(parser),
        Some(Token::Super) => return parse_super(parser),
        Some(Token::LeftParen) => {
            parser.advance();
            let (expression, depth) = parser.parse_expression_with_depth(0)?;
            parser.expect(&Token::RightParen, "parenthesized expression")?;
            return Ok((expression, depth));
        }
        _ => return Err(parser.error("Expected an expression", "expression")),
    };
    parser.advance();
    Ok((expression, 1))
}

/// Parses `new Callee(arguments)`. The callee takes member accesses but no
/// calls, so `new a.B().c()` constructs `a.B` and then calls `c` on the
/// instance. The argument list is optional.
fn parse_new(parser: &mut Parser<'_>) -> ParseResult<(Expr, usize)> {
    parser.expect(&Token::New, "new expression")?;
    parser.nested("new expression", |parser| {
        let (callee, callee_depth) = primary(parser)?;
        let (callee, callee_depth) = suffix(parser, callee, callee_depth, false)?;
        let (arguments, arguments_depth) = if parser.consume(&Token::LeftParen) {
            arguments(parser)?
        } else {
            (Vec::new(), 0)
        };
        let depth = parser.node_depth(callee_depth.max(arguments_depth) + 1, "new expression")?;
        Ok((
            Expr::New {
                callee: Box::new(callee),
                arguments,
            },
            depth,
        ))
    })
}

/// Parses `super(arguments)` or `super.name`; bare `super` is a syntax error.
fn parse_super(parser: &mut Parser<'_>) -> ParseResult<(Expr, usize)> {
    if parser.method_depth == 0 {
        return Err(parser.error("`super` is only valid inside a class method", "super"));
    }
    parser.expect(&Token::Super, "super")?;
    if parser.consume(&Token::LeftParen) {
        let (arguments, depth) = arguments(parser)?;
        let depth = parser.node_depth(depth + 1, "super call")?;
        Ok((Expr::SuperCall { arguments }, depth))
    } else if parser.consume(&Token::Dot) {
        let name = property_name(parser)?;
        Ok((Expr::SuperMember { name }, 1))
    } else {
        Err(parser.error("Expected `(` or `.` after `super`", "super"))
    }
}

/// Parses call arguments after `(`, through the closing `)`.
fn arguments(parser: &mut Parser<'_>) -> ParseResult<(Vec<Expr>, usize)> {
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
    Ok((arguments, arguments_depth))
}

/// Parses any chain of call, `.name`, and `[key]` suffixes, left to right.
/// With `allow_calls` false, a `(` ends the chain, as for a `new` callee.
fn suffix(
    parser: &mut Parser<'_>,
    mut callee: Expr,
    mut depth: usize,
    allow_calls: bool,
) -> ParseResult<(Expr, usize)> {
    loop {
        if allow_calls && parser.consume(&Token::LeftParen) {
            let (arguments, arguments_depth) = arguments(parser)?;
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
/// are allowed, so `object.if` and `object.null` are ordinary properties, and
/// so are escaped keywords such as `object.\u0069f`.
pub(in crate::parser) fn property_name(parser: &mut Parser<'_>) -> ParseResult<String> {
    let name = match parser.peek() {
        Some(Token::Identifier(name) | Token::EscapedKeyword(name)) => name.as_str(),
        Some(Token::Let) => "let",
        Some(Token::Var) => "var",
        Some(Token::Const) => "const",
        Some(Token::If) => "if",
        Some(Token::Else) => "else",
        Some(Token::While) => "while",
        Some(Token::Function) => "function",
        Some(Token::Return) => "return",
        Some(Token::Class) => "class",
        Some(Token::Extends) => "extends",
        Some(Token::New) => "new",
        Some(Token::This) => "this",
        Some(Token::Super) => "super",
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
