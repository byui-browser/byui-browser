use super::super::{ParseResult, Parser};
use crate::ast::Statement;
use crate::lexer::Token;

pub(super) fn parse(parser: &mut Parser<'_>) -> ParseResult<Statement> {
    let is_if = parser.consume(&Token::If);
    if !is_if {
        parser.expect(&Token::While, "while statement")?;
    }
    parser.expect(&Token::LeftParen, "condition")?;
    let condition = parser.parse_expression(0)?;
    parser.expect(&Token::RightParen, "condition")?;
    let body = Box::new(parser.parse_statement()?);
    if is_if {
        let else_branch = if parser.consume(&Token::Else) {
            Some(Box::new(parser.parse_statement()?))
        } else {
            None
        };
        Ok(Statement::If {
            condition,
            then_branch: body,
            else_branch,
        })
    } else {
        Ok(Statement::While { condition, body })
    }
}
