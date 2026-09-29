#[derive(Debug, Default, Clone, PartialEq)]
pub struct Program {
    pub body: Vec<Statement>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// `42`
    Number(f64),
    String(String),
    Boolean(bool),
    Null,
    Undefined,
    Identifier(String),
    Unary {
        operator: UnaryOperator,
        operand: Box<Expr>,
    },
    Binary {
        left: Box<Expr>,
        operator: BinaryOperator,
        right: Box<Expr>,
    },
    Logical {
        left: Box<Expr>,
        operator: LogicalOperator,
        right: Box<Expr>,
    },
    Assign {
        name: String,
        value: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        arguments: Vec<Expr>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Block(Vec<Statement>),
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
    FunctionDeclaration {
        name: String,
        params: Vec<String>,
        body: Vec<Statement>,
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
    Remainder,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Equal,
    NotEqual,
    StrictEqual,
    StrictNotEqual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalOperator {
    And,
    Or,
}
