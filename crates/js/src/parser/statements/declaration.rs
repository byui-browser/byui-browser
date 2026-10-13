use super::super::{ParseResult, Parser};
use crate::ast::{Statement, VarKind};
use crate::lexer::Token;

pub(super) fn parse(parser: &mut Parser<'_>) -> ParseResult<Statement> {
    let kind = match parser.advance() {
        Some(Token::Var) => VarKind::Var,
        Some(Token::Let) => VarKind::Let,
        Some(Token::Const) => VarKind::Const,
        _ => return Err(parser.error("Expected a declaration keyword", "variable declaration")),
    };
    let name = parser.identifier("variable declaration")?;
    let init = if parser.consume(&Token::Assign) {
        Some(parser.parse_expression(0)?)
    } else {
        None
    };
    if kind == VarKind::Const && init.is_none() {
        return Err(parser.error("Constant requires an initializer", "variable declaration"));
    }
    parser.terminator("variable declaration")?;
    Ok(Statement::VariableDeclaration { kind, name, init })
}
