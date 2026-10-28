use super::super::expressions::property_name;
use super::super::{ParseResult, Parser};
use super::{block, function};
use crate::ast::{ClassDefinition, MethodDefinition, Statement};
use crate::lexer::Token;

/// Highest binding power, so `extends` takes a call/member expression and
/// stops before the class body's `{`.
const SUPERCLASS_PRECEDENCE: u8 = u8::MAX;

pub(super) fn parse(parser: &mut Parser<'_>) -> ParseResult<Statement> {
    parser.expect(&Token::Class, "class declaration")?;
    let name = parser.identifier("class declaration")?;
    let superclass = if parser.consume(&Token::Extends) {
        Some(parser.parse_expression(SUPERCLASS_PRECEDENCE)?)
    } else {
        None
    };
    parser.expect(&Token::LeftBrace, "class body")?;
    let mut constructor = None;
    let mut methods = Vec::new();
    while !parser.consume(&Token::RightBrace) {
        if parser.peek().is_none() {
            return Err(parser.error("Expected RightBrace", "class body"));
        }
        // Stray semicolons between members are allowed.
        if parser.consume(&Token::Semicolon) {
            continue;
        }
        let method = parse_method(parser)?;
        if method.name == "constructor" {
            if constructor.is_some() {
                return Err(parser.error("A class may only have one constructor", "class body"));
            }
            constructor = Some(method);
        } else {
            methods.push(method);
        }
    }
    Ok(Statement::ClassDeclaration(Box::new(ClassDefinition {
        name,
        superclass,
        constructor,
        methods,
    })))
}

fn parse_method(parser: &mut Parser<'_>) -> ParseResult<MethodDefinition> {
    let name = property_name(parser, "class body")?;
    if parser.peek() != Some(&Token::LeftParen) {
        return Err(parser.error(
            "Expected a method; static members, accessors, and fields are unsupported",
            "class body",
        ));
    }
    let params = function::parse_params(parser)?;
    parser.function_depth += 1;
    parser.method_depth += 1;
    let body = block::parse(parser);
    parser.method_depth -= 1;
    parser.function_depth -= 1;
    Ok(MethodDefinition {
        name,
        params,
        body: body?,
    })
}
