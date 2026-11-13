use super::super::{ParseResult, Parser};
use crate::ast::Statement;
use crate::lexer::Token;

pub(super) fn parse(parser: &mut Parser<'_>) -> ParseResult<Statement> {
    if parser.consume(&Token::For) {
        return parse_for(parser);
    }
    let is_if = parser.consume(&Token::If);
    if !is_if {
        parser.expect(&Token::While, "while statement")?;
    }
    parser.expect(&Token::LeftParen, "condition")?;
    let condition = parser.parse_sequence()?;
    parser.expect(&Token::RightParen, "condition")?;
    if !is_if {
        parser.loops.push(parser.function_depth);
    }
    let body = body_statement(parser);
    if !is_if {
        parser.loops.pop();
    }
    let body = Box::new(body?);
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

fn parse_for(parser: &mut Parser<'_>) -> ParseResult<Statement> {
    parser.expect(&Token::LeftParen, "for statement")?;
    let init = if parser.consume(&Token::Semicolon) {
        None
    } else if matches!(parser.peek(), Some(Token::Var | Token::Let | Token::Const)) {
        Some(Box::new(super::declaration::parse_for(parser)?))
    } else {
        let expression = parser.parse_sequence()?;
        parser.expect(&Token::Semicolon, "for initializer")?;
        Some(Box::new(Statement::Expression(expression)))
    };
    let condition = if parser.consume(&Token::Semicolon) {
        None
    } else {
        let expression = parser.parse_sequence()?;
        parser.expect(&Token::Semicolon, "for condition")?;
        Some(expression)
    };
    let update = if parser.consume(&Token::RightParen) {
        None
    } else {
        let expression = parser.parse_sequence()?;
        parser.expect(&Token::RightParen, "for update")?;
        Some(expression)
    };
    parser.loops.push(parser.function_depth);
    let body = body_statement(parser);
    parser.loops.pop();
    Ok(Statement::For {
        init,
        condition,
        update,
        body: Box::new(body?),
    })
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
