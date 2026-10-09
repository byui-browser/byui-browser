//! Statement dispatch and termination for the supported grammar.
mod block;
mod control_flow;
mod declaration;
mod function;
mod return_statement;

use super::{ParseResult, Parser};
use crate::ast::Statement;
use crate::lexer::Token;

pub(super) fn parse(parser: &mut Parser<'_>) -> ParseResult<Statement> {
    match parser.peek() {
        Some(Token::LeftBrace) => Ok(Statement::Block(block::parse(parser)?)),
        Some(Token::Let | Token::Var | Token::Const) => declaration::parse(parser),
        Some(Token::If | Token::While) => control_flow::parse(parser),
        Some(Token::Function) => function::parse(parser),
        Some(Token::Return) => return_statement::parse(parser),
        _ => {
            let expression = parser.parse_expression(0)?;
            parser.terminator("expression statement")?;
            Ok(Statement::Expression(expression))
        }
    }
}
