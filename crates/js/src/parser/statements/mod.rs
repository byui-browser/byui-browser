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
        Some(Token::Semicolon) => {
            parser.advance();
            Ok(Statement::Empty)
        }
        Some(Token::LeftBrace) => Ok(Statement::Block(block::parse(parser)?)),
        Some(Token::Let | Token::Var | Token::Const) => declaration::parse(parser),
        Some(Token::If | Token::While | Token::For | Token::Do | Token::Switch) => {
            control_flow::parse(parser)
        }
        Some(Token::Function) => function::parse(parser),
        Some(Token::Class) => class::parse(parser),
        Some(Token::Return) => return_statement::parse(parser),
        Some(Token::Throw) => exceptions::parse_throw(parser),
        Some(Token::Try) => exceptions::parse_try(parser),
        Some(Token::Break | Token::Continue) => exceptions::parse_jump(parser),
        Some(Token::Identifier(_)) if parser.peek_at(1) == Some(&Token::Colon) => {
            let label = parser.identifier("label")?;
            parser.expect(&Token::Colon, "labeled statement")?;
            let loop_target = matches!(parser.peek(), Some(Token::While | Token::For | Token::Do));
            parser.push_label(label.clone(), loop_target)?;
            let body = parser.parse_statement();
            parser.pop_label();
            Ok(Statement::Labeled {
                label,
                body: Box::new(body?),
            })
        }
        _ => {
            let expression = parser.parse_sequence()?;
            parser.terminator("expression statement")?;
            Ok(Statement::Expression(expression))
        }
    }
}
