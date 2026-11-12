//! Parsing for `throw`, `try`/`catch`/`finally`, `break`, and `continue`.

use super::super::{ParseResult, Parser};
use crate::ast::{CatchClause, Statement};
use crate::lexer::Token;

/// Parses `throw expression;`. A line terminator after `throw` is an error,
/// because automatic semicolon insertion would otherwise throw nothing.
pub(super) fn parse_throw(parser: &mut Parser<'_>) -> ParseResult<Statement> {
    parser.expect(&Token::Throw, "throw statement")?;
    if parser.line_terminator_before_current() {
        return Err(parser.error("Illegal newline after throw", "throw statement"));
    }
    let value = parser.parse_expression(0)?;
    parser.terminator("throw statement")?;
    Ok(Statement::Throw(value))
}

/// Parses `try` with a `catch` clause, a `finally` block, or both.
pub(super) fn parse_try(parser: &mut Parser<'_>) -> ParseResult<Statement> {
    parser.expect(&Token::Try, "try statement")?;
    let block = super::block::parse(parser)?;
    let handler = if parser.consume(&Token::Catch) {
        let param = if parser.consume(&Token::LeftParen) {
            let name = parser.identifier("catch parameter")?;
            parser.expect(&Token::RightParen, "catch parameter")?;
            Some(name)
        } else {
            None
        };
        Some(CatchClause {
            param,
            body: super::block::parse(parser)?,
        })
    } else {
        None
    };
    let finalizer = if parser.consume(&Token::Finally) {
        Some(super::block::parse(parser)?)
    } else {
        None
    };
    if handler.is_none() && finalizer.is_none() {
        return Err(parser.error("Missing catch or finally after try", "try statement"));
    }
    Ok(Statement::Try {
        block,
        handler,
        finalizer,
    })
}

/// Parses `break;` or `continue;`, which must sit inside a `while` loop of
/// the same function.
pub(super) fn parse_jump(parser: &mut Parser<'_>) -> ParseResult<Statement> {
    let is_break = parser.consume(&Token::Break);
    if !is_break {
        parser.expect(&Token::Continue, "continue statement")?;
    }
    let context = if is_break {
        "break statement"
    } else {
        "continue statement"
    };
    if parser.loops.last() != Some(&parser.function_depth) {
        return Err(parser.error(
            if is_break {
                "Break requires an enclosing loop"
            } else {
                "Continue requires an enclosing loop"
            },
            context,
        ));
    }
    parser.terminator(context)?;
    Ok(if is_break {
        Statement::Break
    } else {
        Statement::Continue
    })
}
