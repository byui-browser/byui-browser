#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// `42`
    Number(f64),
    String(String),
    Boolean(bool),
    Null,
    Undefined,
    Unary {
        operator: UnaryOperator,
        operand: Box<Expr>,
    },
    Binary {
        left: Box<Expr>,
        operator: BinaryOperator,
        right: Box<Expr>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Expression(Expr),
    VariableDeclaration {
        kind: VarKind,
        name: String,
        init: Option<Expr>,
    },
    If {
        condition: Expr,
        then_branch: Box<Statement>,
        else_branch: Option<Box<Statement>>,
    },
    While {
        condition: Expr,
        body: Box<Statement>,
    },
    Return(Option<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarKind {
    Var,
    Let,
    Const,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOperator {
    /// `-x`
    Negate,
    /// `!x`
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
}
