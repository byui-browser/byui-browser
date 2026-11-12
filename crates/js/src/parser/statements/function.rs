use super::super::{ParseResult, Parser};
use crate::ast::Statement;
use crate::lexer::Token;

pub(super) fn parse(parser: &mut Parser<'_>) -> ParseResult<Statement> {
    parser.expect(&Token::Function, "function declaration")?;
    let name = parser.identifier("function declaration")?;
    let params = parse_params(parser)?;
    parser.function_depth += 1;
    // A plain function nested in a method has no home object for `super`.
    let method_depth = std::mem::take(&mut parser.method_depth);
    let body = super::block::parse(parser);
    parser.method_depth = method_depth;
    parser.function_depth -= 1;
    Ok(Statement::FunctionDeclaration {
        name,
        params,
        body: body?,
    })
}

/// Parses a parenthesized parameter list, rejecting duplicate names.
pub(super) fn parse_params(parser: &mut Parser<'_>) -> ParseResult<Vec<String>> {
    parser.expect(&Token::LeftParen, "function parameters")?;
    let mut params = Vec::new();
    if !parser.consume(&Token::RightParen) {
        loop {
            if matches!(parser.peek(), Some(Token::Identifier(name)) if params.contains(name)) {
                return Err(parser.error("Duplicate parameter name", "function parameters"));
            }
            params.push(parser.identifier("function parameters")?);
            if !parser.consume(&Token::Comma) {
                break;
            }
        }
        parser.expect(&Token::RightParen, "function parameters")?;
    }
    Ok(params)
}
