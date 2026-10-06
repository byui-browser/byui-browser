use super::super::{ParseResult, Parser};
use crate::ast::Statement;
use crate::lexer::Token;

pub(super) fn parse(parser: &mut Parser<'_>) -> ParseResult<Vec<Statement>> {
    parser.expect(&Token::LeftBrace, "block")?;
    let mut body = Vec::new();
    while !parser.consume(&Token::RightBrace) {
        if parser.peek().is_none() {
            return Err(parser.error("Expected RightBrace", "block"));
        }
        body.push(parser.parse_statement()?);
    }
    Ok(body)
}
