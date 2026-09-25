use crate::ast::{BinaryOperator, Expr};

/// Evaluates a numeric expression from the AST.
///
/// Non-numeric AST nodes are reserved for the full evaluator and are rejected
/// explicitly until that evaluator is implemented.
pub fn evaluate(expr: &Expr) -> f64 {
    match expr {
        Expr::Number(number) => *number,
        Expr::Binary {
            left,
            operator,
            right,
        } => {
            let left = evaluate(left);
            let right = evaluate(right);
            match operator {
                BinaryOperator::Add => left + right,
                BinaryOperator::Subtract => left - right,
                BinaryOperator::Multiply => left * right,
                BinaryOperator::Divide => left / right,
                BinaryOperator::Remainder => left % right,
                BinaryOperator::Less => (left < right) as u8 as f64,
                BinaryOperator::LessEqual => (left <= right) as u8 as f64,
                BinaryOperator::Greater => (left > right) as u8 as f64,
                BinaryOperator::GreaterEqual => (left >= right) as u8 as f64,
                BinaryOperator::Equal | BinaryOperator::StrictEqual => {
                    (left == right) as u8 as f64
                }
                BinaryOperator::NotEqual | BinaryOperator::StrictNotEqual => {
                    (left != right) as u8 as f64
                }
            }
        }
        Expr::Unary { .. }
        | Expr::String(..)
        | Expr::Boolean(..)
        | Expr::Null
        | Expr::Undefined
        | Expr::Identifier(..)
        | Expr::Logical { .. }
        | Expr::Assign { .. }
        | Expr::Call { .. } => panic!("expression is not supported by the numeric evaluator"),
    }
}
