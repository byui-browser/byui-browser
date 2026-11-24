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
    let declaration = finish_list(parser, kind, name, init)?;
    parser.terminator("variable declaration")?;
    Ok(declaration)
}

pub(super) fn finish_list(
    parser: &mut Parser<'_>,
    kind: VarKind,
    name: String,
    init: Option<crate::ast::Expr>,
) -> ParseResult<Statement> {
    let mut declarations = Vec::new();
    let mut name = name;
    let mut init = init;
    loop {
        if kind == VarKind::Const && init.is_none() {
            return Err(parser.error("Constant requires an initializer", "variable declaration"));
        }
        declarations.push(Statement::VariableDeclaration { kind, name, init });
        if !parser.consume(&Token::Comma) {
            break;
        }
        name = parser.identifier("variable declaration")?;
        init = if parser.consume(&Token::Assign) {
            Some(parser.parse_expression(0)?)
        } else {
            None
        };
    }
    Ok(if declarations.len() == 1 {
        declarations.pop().expect("one declaration")
    } else {
        Statement::VariableDeclarationList(declarations)
    })
}
