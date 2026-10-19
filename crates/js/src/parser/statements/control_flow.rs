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
    let body = Box::new(body_statement(parser)?);
    if is_if {
        let else_branch = if parser.consume(&Token::Else) {
            Some(Box::new(body_statement(parser)?))
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

/// Parses a single-statement body, which may not be a lexical or function
/// declaration because it has no block scope to hold the binding.
fn body_statement(parser: &mut Parser<'_>) -> ParseResult<Statement> {
    if matches!(
        parser.peek(),
        Some(Token::Let | Token::Const | Token::Function)
    ) {
        return Err(parser.error(
            "Declaration cannot appear in a single-statement context",
            "statement body",
        ));
    }
    parser.parse_statement()
}
