//! Owned abstract syntax tree produced by the parser and run by the runtime.

/// An owned JavaScript program in source order.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Program {
    /// Owned statements executed in source order.
    pub body: Vec<Statement>,
}

/// An owned expression tree; grouping and precedence are encoded by nesting.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Numeric literal stored as a 64-bit floating-point value.
    Number(f64),
    /// Owned decoded string literal.
    String(String),
    /// Boolean literal.
    Boolean(bool),
    /// The null literal.
    Null,
    /// The undefined literal.
    Undefined,
    /// Owned name to resolve in the execution environment.
    Identifier(String),
    /// Unary operation on one operand.
    Unary {
        /// Operation to apply to the operand.
        operator: UnaryOperator,
        /// Owned operand expression.
        operand: Box<Expr>,
    },
    /// Eager binary operation with ordered operands.
    Binary {
        /// Owned left operand, evaluated first.
        left: Box<Expr>,
        /// Arithmetic or comparison operation.
        operator: BinaryOperator,
        /// Owned right operand.
        right: Box<Expr>,
    },
    /// Logical operation whose right operand may be skipped by short-circuit evaluation.
    Logical {
        /// Owned left operand, evaluated first.
        left: Box<Expr>,
        /// Short-circuit operation controlling right-operand evaluation.
        operator: LogicalOperator,
        /// Owned right operand.
        right: Box<Expr>,
    },
    /// Assignment to an identifier, yielding the assigned value.
    Assign {
        /// Owned identifier spelling.
        name: String,
        /// Owned expression supplying the assigned value.
        value: Box<Expr>,
    },
    /// Function invocation; the callee may itself be a call.
    Call {
        /// Owned expression resolving the callable value.
        callee: Box<Expr>,
        /// Owned arguments in evaluation order.
        arguments: Vec<Expr>,
    },
}

/// An owned statement in the supported program grammar.
#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    /// Owned statements enclosed in braces, in source order.
    Block(Vec<Statement>),
    /// Expression executed for its effects and completion value.
    Expression(Expr),
    /// Declaration of one named variable.
    VariableDeclaration {
        /// Declaration keyword and its binding semantics.
        kind: VarKind,
        /// Owned identifier spelling.
        name: String,
        /// Optional initializer; constants require one when parsed.
        init: Option<Expr>,
    },
    /// Conditional statement with an optional alternative.
    If {
        /// Owned expression tested for truthiness.
        condition: Expr,
        /// Statement executed when the condition is truthy.
        then_branch: Box<Statement>,
        /// Optional statement executed when the condition is falsy.
        else_branch: Option<Box<Statement>>,
    },
    /// Loop that tests its condition before each iteration.
    While {
        /// Owned expression tested for truthiness.
        condition: Expr,
        /// Owned statement executed on each iteration.
        body: Box<Statement>,
    },
    /// Named function declaration with an owned body.
    FunctionDeclaration {
        /// Owned identifier spelling.
        name: String,
        /// Owned parameter names in argument order.
        params: Vec<String>,
        /// Owned function statements in source order.
        body: Vec<Statement>,
    },
    /// Function return with an optional value; absence means undefined.
    Return(Option<Expr>),
}

/// Keyword used to declare a binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarKind {
    /// Function-scoped variable declaration (`var`).
    Var,
    /// Block-scoped mutable declaration (`let`).
    Let,
    /// Block-scoped immutable declaration (`const`).
    Const,
}

/// Supported single-operand operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOperator {
    /// `-x`
    Negate,
    /// `!x`
    Not,
    /// `~x`
    BitwiseNot,
}

/// Arithmetic, bitwise, and comparison operations represented by the AST.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOperator {
    /// `+` operator.
    Add,
    /// `-` operator.
    Subtract,
    /// `*` operator.
    Multiply,
    /// `/` operator.
    Divide,
    /// `%` operator; the result takes the sign of the dividend.
    Remainder,
    /// `&` operator on 32-bit integers.
    BitwiseAnd,
    /// `|` operator on 32-bit integers.
    BitwiseOr,
    /// `^` operator on 32-bit integers.
    BitwiseXor,
    /// `<<` operator; the shift count is taken modulo 32.
    LeftShift,
    /// `>>` sign-propagating operator; the shift count is taken modulo 32.
    SignedRightShift,
    /// `>>>` zero-fill operator; the shift count is taken modulo 32.
    UnsignedRightShift,
    /// `<` operator.
    Less,
    /// `<=` operator.
    LessEqual,
    /// `>` operator.
    Greater,
    /// `>=` operator.
    GreaterEqual,
    /// `==` operator.
    Equal,
    /// `!=` operator.
    NotEqual,
    /// `===` operator.
    StrictEqual,
    /// `!==` operator.
    StrictNotEqual,
}

/// Operations represented separately to preserve short-circuit structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalOperator {
    /// `&&`: evaluate the right operand only if the left is truthy.
    And,
    /// `||`: evaluate the right operand only if the left is falsy.
    Or,
}
