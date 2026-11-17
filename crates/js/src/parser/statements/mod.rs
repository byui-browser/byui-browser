//! Statement dispatch and termination for the supported grammar.
mod block;
mod class;
mod control_flow;
mod declaration;
mod exceptions;
mod function;
mod return_statement;

pub(in crate::parser) use block::parse_arrow_block;

use super::{ParseResult, Parser};
use crate::ast::Statement;
use crate::lexer::Token;

pub(super) fn parse(parser: &mut Parser<'_>) -> ParseResult<Statement> {
    if log::log_enabled!(log::Level::Trace) {
        log::trace!("parsing statement starting at {:?}", parser.peek());
    }
    match parser.peek() {
        Some(Token::LeftBrace) => Ok(Statement::Block(block::parse(parser)?)),
        Some(Token::Let | Token::Var | Token::Const) => declaration::parse(parser),
        Some(Token::If | Token::While | Token::For) => control_flow::parse(parser),
        Some(Token::Function) => function::parse(parser),
        Some(Token::Class) => class::parse(parser),
        Some(Token::Return) => return_statement::parse(parser),
        Some(Token::Throw) => exceptions::parse_throw(parser),
        Some(Token::Try) => exceptions::parse_try(parser),
        Some(Token::Break | Token::Continue) => exceptions::parse_jump(parser),
        _ => {
            let expression = parser.parse_sequence()?;
            parser.terminator("expression statement")?;
            Ok(Statement::Expression(expression))
        }
    }
}
