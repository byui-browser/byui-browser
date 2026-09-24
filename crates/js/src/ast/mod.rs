/// Expressions produced by the parser.
#[derive(Debug, PartialEq)]
pub enum Expr {
    Number(f64),
    Binary {
        left: Box<Expr>,
        operator: BinaryOperator,
        right: Box<Expr>,
    },
}

/// Operators that combine two expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
}
