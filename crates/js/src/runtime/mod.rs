use crate::ast::{BinaryOperator, Expr};

/// Evaluates an expression from the AST and returns its numeric result.
/// Evaluates an expression from the AST and returns its value.
pub fn evaluate(expr: &Expr) -> Value {
    match expr {
        Expr::Number(number) => Value::Number(*number),
        Expr::String(text) => Value::String(text.clone()),
        Expr::Boolean(flag) => Value::Boolean(*flag),
        Expr::Null => Value::Null,
        Expr::Undefined => Value::Undefined,
        Expr::Unary { operator, operand } => {
            let value = evaluate(operand);
            match operator {
                UnaryOperator::Negate => Value::Number(-to_number(&value)),
                UnaryOperator::Not => Value::Boolean(!is_truthy(&value)),
            }
        }
        Expr::Binary {
            left,
            operator,
            right,
        } => {
            let left = to_number(&evaluate(left));
            let right = to_number(&evaluate(right));
            match operator {
                BinaryOperator::Add => Value::Number(left + right),
                BinaryOperator::Subtract => Value::Number(left - right),
                BinaryOperator::Multiply => Value::Number(left * right),
                BinaryOperator::Divide => Value::Number(left / right),
                BinaryOperator::Remainder => Value::Number(left % right),
                BinaryOperator::Less => Value::Boolean(left < right),
                BinaryOperator::LessEqual => Value::Boolean(left <= right),
                BinaryOperator::Greater => Value::Boolean(left > right),
                BinaryOperator::GreaterEqual => Value::Boolean(left >= right),
                BinaryOperator::Equal | BinaryOperator::StrictEqual => {
                    Value::Boolean(left == right)
                }
                BinaryOperator::NotEqual | BinaryOperator::StrictNotEqual => {
                    Value::Boolean(left != right)
                }
            }
        }
        Expr::Logical {
            left,
            operator,
            right,
        } => {
            let left = evaluate(left);
            match operator {
                LogicalOperator::And if !is_truthy(&left) => left,
                LogicalOperator::Or if is_truthy(&left) => left,
                _ => evaluate(right),
            }
        }
        Expr::Identifier(_) | Expr::Assign { .. } | Expr::Call { .. } => {
            todo!("variables and function calls need a Realm")
        }
    }
}
