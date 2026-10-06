use super::super::{ParseResult, Parser};
use crate::ast::Statement;
use crate::lexer::Token;

pub(super) fn parse(parser: &mut Parser<'_>) -> ParseResult<Statement> {
    parser.expect(&Token::Function, "function declaration")?;
    let name = parser.identifier("function declaration")?;
    parser.expect(&Token::LeftParen, "function parameters")?;
    let mut params = Vec::new();
    if !parser.consume(&Token::RightParen) {
        loop {
            params.push(parser.identifier("function parameters")?);
            if !parser.consume(&Token::Comma) {
                break;
            }
        }
        parser.expect(&Token::RightParen, "function parameters")?;
    }
    parser.function_depth += 1;
    let body = super::block::parse(parser);
    parser.function_depth -= 1;
    Ok(Statement::FunctionDeclaration {
        name,
        params,
        body: body?,
    })
}
