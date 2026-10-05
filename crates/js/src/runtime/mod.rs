use std::collections::HashMap;

use crate::ast::{
    BinaryOperator, Expr, LogicalOperator, Program, Statement, UnaryOperator, VarKind,
};
use crate::{JsError, JsResult, Value};

#[derive(Clone, Debug)]
struct Binding {
    value: Value,
    mutable: bool,
}

/// A chain of lexical environments used while executing a JavaScript program.
#[derive(Debug, Default)]
pub struct Environment {
    scopes: Vec<HashMap<String, Binding>>,
}

impl Environment {
    /// Creates an environment with one global lexical scope.
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }
    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn declare(&mut self, name: &str, value: Value, kind: VarKind) -> JsResult<()> {
        let scope = self
            .scopes
            .last_mut()
            .expect("environment always has a scope");
        if scope.contains_key(name) {
            return Err(JsError::new(format!("`{name}` has already been declared")));
        }
        scope.insert(
            name.to_owned(),
            Binding {
                value,
                mutable: !matches!(kind, VarKind::Const),
            },
        );
        Ok(())
    }

    /// Reads the nearest binding with `name`.
    pub fn get(&self, name: &str) -> JsResult<Value> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name))
            .map(|binding| binding.value.clone())
            .ok_or_else(|| JsError::new(format!("`{name}` is not defined")))
    }

    /// Updates the nearest mutable binding with `name`.
    pub fn set(&mut self, name: &str, value: Value) -> JsResult<Value> {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(binding) = scope.get_mut(name) {
                if !binding.mutable {
                    return Err(JsError::new(format!("Assignment to constant `{name}`")));
                }
                binding.value = value.clone();
                return Ok(value);
            }
        }
        Err(JsError::new(format!("`{name}` is not defined")))
    }
}

/// Evaluates a parsed program with a fresh lexical environment.
pub fn evaluate_program(program: &Program) -> JsResult<Value> {
    let mut environment = Environment::new();
    execute_statements(&mut environment, &program.body)
}

fn execute_statements(environment: &mut Environment, statements: &[Statement]) -> JsResult<Value> {
    let mut result = Value::Undefined;
    for statement in statements {
        result = execute_statement(environment, statement)?;
    }
    Ok(result)
}

fn execute_statement(environment: &mut Environment, statement: &Statement) -> JsResult<Value> {
    match statement {
        Statement::Expression(expression) => evaluate_in(expression, environment),
        Statement::VariableDeclaration { kind, name, init } => {
            let value = init.as_ref().map_or(Ok(Value::Undefined), |expression| {
                evaluate_in(expression, environment)
            })?;
            environment.declare(name, value, *kind)?;
            Ok(Value::Undefined)
        }
        Statement::Block(statements) => {
            environment.push_scope();
            let result = execute_statements(environment, statements);
            environment.pop_scope();
            result
        }
        _ => Err(JsError::new(
            "statement is not supported by the tree-walk interpreter",
        )),
    }
}

fn evaluate_in(expression: &Expr, environment: &mut Environment) -> JsResult<Value> {
    match expression {
        Expr::Identifier(name) => environment.get(name),
        Expr::Assign { name, value } => {
            let value = evaluate_in(value, environment)?;
            environment.set(name, value)
        }
        Expr::Number(number) => Ok(Value::Number(*number)),
        Expr::String(text) => Ok(Value::String(text.clone())),
        Expr::Boolean(value) => Ok(Value::Boolean(*value)),
        Expr::Null => Ok(Value::Null),
        Expr::Undefined => Ok(Value::Undefined),
        Expr::Unary { operator, operand } => {
            let value = evaluate_in(operand, environment)?;
            Ok(match operator {
                UnaryOperator::Negate => {
                    Value::Number(to_number(&value).map_or(f64::NAN, |number| -number))
                }
                UnaryOperator::Not => Value::Boolean(!is_truthy(&value)),
            })
        }
        Expr::Binary {
            left,
            operator,
            right,
        } => {
            let left = evaluate_in(left, environment)?;
            let right = evaluate_in(right, environment)?;
            binary(*operator, left, right)
        }
        Expr::Logical {
            left,
            operator,
            right,
        } => {
            let left = evaluate_in(left, environment)?;
            let use_right = match operator {
                LogicalOperator::And => is_truthy(&left),
                LogicalOperator::Or => !is_truthy(&left),
            };
            if use_right {
                evaluate_in(right, environment)
            } else {
                Ok(left)
            }
        }
        Expr::Call { .. } => Err(JsError::new(
            "function calls are not supported by the tree-walk interpreter",
        )),
    }
}

fn binary(operator: BinaryOperator, left: Value, right: Value) -> JsResult<Value> {
    let (left, right) = (to_number(&left)?, to_number(&right)?);
    Ok(match operator {
        BinaryOperator::Add => Value::Number(left + right),
        BinaryOperator::Subtract => Value::Number(left - right),
        BinaryOperator::Multiply => Value::Number(left * right),
        BinaryOperator::Divide => Value::Number(left / right),
        BinaryOperator::Remainder => Value::Number(left % right),
        BinaryOperator::Less => Value::Boolean(left < right),
        BinaryOperator::LessEqual => Value::Boolean(left <= right),
        BinaryOperator::Greater => Value::Boolean(left > right),
        BinaryOperator::GreaterEqual => Value::Boolean(left >= right),
        BinaryOperator::Equal | BinaryOperator::StrictEqual => Value::Boolean(left == right),
        BinaryOperator::NotEqual | BinaryOperator::StrictNotEqual => Value::Boolean(left != right),
    })
}

fn to_number(value: &Value) -> JsResult<f64> {
    match value {
        Value::Number(number) => Ok(*number),
        Value::Boolean(true) => Ok(1.0),
        Value::Boolean(false) | Value::Null => Ok(0.0),
        Value::Undefined => Ok(f64::NAN),
        Value::String(text) => text
            .trim()
            .parse()
            .map_err(|_| JsError::new("cannot convert string to number")),
    }
}
fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Undefined | Value::Null => false,
        Value::Boolean(value) => *value,
        Value::Number(value) => *value != 0.0 && !value.is_nan(),
        Value::String(value) => !value.is_empty(),
    }
}

/// Evaluates an expression without declarations, using a fresh environment.
pub fn evaluate(expression: &Expr) -> Value {
    evaluate_in(expression, &mut Environment::new()).unwrap_or(Value::Undefined)
}
