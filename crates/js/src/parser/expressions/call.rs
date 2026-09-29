// TODO(parser): Add call-expression token parsing once the lexer exposes
// identifiers, parentheses, commas, and argument tokens.

use crate::ast::Expr;

pub(super) fn build(callee: Expr, arguments: Vec<Expr>) -> Expr {
    Expr::Call {
        callee: Box::new(callee),
        arguments,
    }
}
