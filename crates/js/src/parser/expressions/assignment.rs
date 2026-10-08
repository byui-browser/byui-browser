use crate::ast::Expr;

pub(super) fn build(name: impl Into<String>, value: Expr) -> Expr {
    Expr::Assign {
        name: name.into(),
        value: Box::new(value),
    }
}
