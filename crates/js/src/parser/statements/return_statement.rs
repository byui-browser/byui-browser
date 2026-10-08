use super::super::{ParseResult, Parser};
use crate::ast::Statement;
use crate::lexer::Token;

pub(super) fn parse(parser: &mut Parser<'_>) -> ParseResult<Statement> {
    if parser.function_depth == 0 {
        return Err(parser.error("Return requires an enclosing function", "return statement"));
    }
    parser.expect(&Token::Return, "return statement")?;
    let value = if matches!(
        parser.peek(),
        None | Some(Token::Semicolon | Token::RightBrace)
    ) {
        None
    } else {
        Some(parser.parse_expression(0)?)
    };
    parser.terminator("return statement")?;
    Ok(Statement::Return(value))
}
