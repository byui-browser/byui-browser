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
        Some(Token::LeftBrace) => return super::object::parse(parser),
        Some(Token::LeftBracket) => return parse_array(parser),
        Some(Token::Template { .. }) => return parse_template(parser),
        // A `/` where an operand must start would begin a regular expression
        // literal, which the engine has no value type for.
        Some(Token::Divide | Token::SlashAssign) => {
            return Err(parser.error(
                "Regular expression literals are not supported",
                "regular expression literal",
            ));
        }
        Some(Token::LeftParen) => {
            parser.advance();
            let (expression, depth) = parser.parse_sequence_with_depth()?;
            parser.expect(&Token::RightParen, "parenthesized expression")?;
            return Ok((expression, depth));
        }
        _ => return Err(parser.error("Expected an expression", "expression")),
    };
    parser.advance();
    Ok((expression, 1))
}

fn parse_array(parser: &mut Parser<'_>) -> ParseResult<(Expr, usize)> {
    parser.expect(&Token::LeftBracket, "array literal")?;
    let mut elements = Vec::new();
    let mut depth = 0;
    while !parser.consume(&Token::RightBracket) {
        if parser.consume(&Token::Comma) {
            elements.push(None);
            continue;
        }
        let (element, element_depth) = argument(parser)?;
        depth = depth.max(element_depth);
        elements.push(Some(element));
        if !parser.consume(&Token::Comma) {
            parser.expect(&Token::RightBracket, "array literal")?;
            break;
        }
    }
    let depth = parser.node_depth(depth + 1, "array literal")?;
    Ok((Expr::Array(elements), depth))
}

/// Parses one array element or call argument: an assignment expression, or
/// `...expression`, which becomes an [`Expr::Spread`].
fn argument(parser: &mut Parser<'_>) -> ParseResult<(Expr, usize)> {
    if parser.consume(&Token::Ellipsis) {
        let (expression, depth) = parser.parse_expression_with_depth(0)?;
        let depth = parser.node_depth(depth + 1, "spread element")?;
        return Ok((Expr::Spread(Box::new(expression)), depth));
    }
    parser.parse_expression_with_depth(0)
}

/// Parses a template literal token, parsing each `${}` substitution as a
/// comma expression with a nested parser.
///
/// An error inside a substitution is reported at the template's own token
/// index, because the substitution's tokens are not part of the outer stream.
fn parse_template<'tokens>(parser: &mut Parser<'tokens>) -> ParseResult<(Expr, usize)> {
    let tokens: &'tokens [Token] = parser.tokens;
    let position = parser.position;
    let Some(Token::Template {
        quasis,
        expressions,
    }) = tokens.get(position)
    else {
        return Err(parser.error("Expected a template literal", "template literal"));
    };
    parser.advance();
    let mut parsed = Vec::with_capacity(expressions.len());
    let mut depth = 0;
    for substitution in expressions {
        let mut inner = Parser::new(substitution, &[]);
        inner.depth = parser.depth;
        inner.function_depth = parser.function_depth;
        inner.method_depth = parser.method_depth;
        inner.loops = parser.loops.clone();
        let result = inner.parse_sequence_with_depth().and_then(|parsed| {
            if inner.peek().is_some() {
                Err(inner.error(
                    "Unexpected token in template substitution",
                    "template literal",
                ))
            } else {
                Ok(parsed)
            }
        });
        match result {
            Ok((expression, expression_depth)) => {
                depth = depth.max(expression_depth);
                parsed.push(expression);
            }
            Err(mut error) => {
                error.token_index = position;
                return Err(error);
            }
        }
    }
    let depth = parser.node_depth(depth + 1, "template literal")?;
    Ok((
        Expr::Template {
            quasis: quasis.clone(),
            expressions: parsed,
        },
        depth,
    ))
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
        let name = property_name(parser, "super")?;
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
            let (argument, argument_depth) = argument(parser)?;
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
    let mut optional = false;
    loop {
        if allow_calls && parser.consume(&Token::LeftParen) {
            let (arguments, arguments_depth) = arguments(parser)?;
            depth = parser.node_depth(depth.max(arguments_depth) + 1, "call arguments")?;
            callee = Expr::Call {
                callee: Box::new(callee),
                arguments,
            };
        } else if parser.peek() == Some(&Token::QuestionDot) {
            if !allow_calls {
                return Err(parser.error(
                    "Optional chaining is not allowed in a `new` callee",
                    "optional chain",
                ));
            }
            parser.advance();
            optional = true;
            if parser.consume(&Token::LeftParen) {
                let (arguments, arguments_depth) = arguments(parser)?;
                depth = parser.node_depth(depth.max(arguments_depth) + 1, "optional call")?;
                callee = Expr::OptionalCall {
                    callee: Box::new(callee),
                    arguments,
                };
            } else if parser.consume(&Token::LeftBracket) {
                let (key, key_depth) = parser.parse_sequence_with_depth()?;
                parser.expect(&Token::RightBracket, "optional computed member access")?;
                depth = parser.node_depth(depth.max(key_depth) + 1, "optional member access")?;
                callee = Expr::OptionalMember {
                    object: Box::new(callee),
                    property: MemberProperty::Computed(Box::new(key)),
                };
            } else {
                let name = property_name(parser, "optional member access")?;
                depth = parser.node_depth(depth + 1, "optional member access")?;
                callee = Expr::OptionalMember {
                    object: Box::new(callee),
                    property: MemberProperty::Named(name),
                };
            }
        } else if parser.consume(&Token::Dot) {
            let name = property_name(parser, "member access")?;
            depth = parser.node_depth(depth + 1, "member access")?;
            callee = Expr::Member {
                object: Box::new(callee),
                property: MemberProperty::Named(name),
            };
        } else if parser.consume(&Token::LeftBracket) {
            let (key, key_depth) = parser.parse_sequence_with_depth()?;
            parser.expect(&Token::RightBracket, "computed member access")?;
            depth = parser.node_depth(depth.max(key_depth) + 1, "computed member access")?;
            callee = Expr::Member {
                object: Box::new(callee),
                property: MemberProperty::Computed(Box::new(key)),
            };
        } else if matches!(parser.peek(), Some(Token::Template { .. })) {
            return Err(parser.error(
                "Tagged template literals are not supported",
                "template literal",
            ));
        } else {
            break;
        }
    }
    if optional {
        depth = parser.node_depth(depth + 1, "optional chain")?;
        callee = Expr::OptionalChain(Box::new(callee));
    }
    Ok((callee, depth))
}

/// Parses the name after `.`, `super.`, a class method, or an object literal
/// key. Like JavaScript's `IdentifierName`, keywords are allowed, so
/// `object.if` and `object.null` are ordinary properties, and so are escaped
/// keywords such as `object.\u0069f`.
pub(in crate::parser) fn property_name(
    parser: &mut Parser<'_>,
    context: &'static str,
) -> ParseResult<String> {
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
        Some(Token::Typeof) => "typeof",
        Some(Token::Void) => "void",
        Some(Token::Delete) => "delete",
        Some(Token::In) => "in",
        Some(Token::InstanceOf) => "instanceof",
        _ => return Err(parser.error("Expected a property name", context)),
    }
    .to_owned();
    parser.advance();
    Ok(name)
}
