use crate::ast::Expr;

/// Builds an assignment to `target`, which the caller has checked is an
/// identifier or member expression.
pub(super) fn build(target: Expr, value: Expr) -> Expr {
    let value = Box::new(value);
    match target {
        Expr::Member { object, property } => Expr::MemberAssign {
            object,
            property,
            value,
        },
        Expr::Identifier(name) => Expr::Assign { name, value },
        _ => unreachable!("assignment target is validated by the caller"),
    }
}
