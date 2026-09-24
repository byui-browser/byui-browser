use crate::ast::{BinaryOperator, Expr};

/// Evaluates an expression from the AST and returns its numeric result.
pub fn evaluate(expr: &Expr) -> f64 {
    match expr {
        Expr::Number(number) => *number,
        Expr::Binary {
            left,
            operator,
            right,
        } => match operator {
            BinaryOperator::Add => evaluate(left) + evaluate(right),
            BinaryOperator::Subtract => evaluate(left) - evaluate(right),
            BinaryOperator::Multiply => evaluate(left) * evaluate(right),
            BinaryOperator::Divide => evaluate(left) / evaluate(right),
        },
    }
}
