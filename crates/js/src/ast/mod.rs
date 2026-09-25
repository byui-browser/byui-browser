<<<<<<< Updated upstream
/// Expressions produced by the parser.
#[derive(Debug, PartialEq)]
=======
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub body: Vec<statement>,
}
>>>>>>> Stashed changes
pub enum Expr {
    Number(f64),
<<<<<<< Updated upstream
=======
    String(String),
    Boolean(bool),
    Null,
    Undefined,
    Identifier(String),
    Unary {
        operator: UnaryOperator,
        operand: Box<Expr>,
    },
>>>>>>> Stashed changes
    Binary {
        left: Box<Expr>,
        operator: BinaryOperator,
        right: Box<Expr>,
    },
    Logical{
        left:Box<Expr>
        operator: LogicalOperator,
        right: Box<Expr>
    },
    Assign {
        name: string,
        value: Box<Expr>
    },
    Call {
        callee: Box<Expr>,
        arguments: Vec<Expr>
    },




}

<<<<<<< Updated upstream
/// Operators that combine two expressions.
=======
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

    FunctionDeclaration{
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

>>>>>>> Stashed changes
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
#cfg[(test)]
mod tests {
    use super::*;
    #[test]
    fn let_with_arithmetic(){
        let statement = Statement::VariableDeclaration{
            kind: Varkind::Let,
            name: "count".to_string(),
            init: Some(Expr::Binary {
                left: Box::new(Expr::Number(1.0)),
                operator: BinaryOperator::Add,
                right: Box::new(Expr::Number(2.0)),
            }),
            };
        assert_eq!(statement.clone(), statement);
    }
    #[test]
    fn if_with_block(){
        let statement = Statement::If {
            condition: Expr::Binary{
                left: Box::new(Expr::Identifier("x".to_string())),
                operator: BinaryOperator::Greater,
                right: Box::new(Expr::Number(5.0)),
            },
            then_branch: Box::new(Statement::Block(vec!
                {Statement::Expression(
                    Expr::Assign {
                    name: "x".to_string(),
                    value: Box::new(Expr::Number(0.0)),
                    },
                )})),
            else_branch: None,
        };
        assert_eq!(statement.clone(), statement);
    }
    #[test]
    fn function_and_call(){
        let program = Program{
            body: vec![
                Statement::FunctionDeclaration{
                    name: "add".to_string(),
                    params: vec!["a".to_string(),
                        "b".to_string()],
                    body: vec!
    [Statement:: Return(Some(Expr::Binary{
                    left: Box::new(Expr::Identifier("a".to_string())),
                    operator: BinaryOperator::Add,
                    right: Box::new(Expr::Identifier("b".to_string())),
    }))],
                },
                Statement::Expression(Expr::Call {
                    callee:
                    Box::new(Expr::Identifier("Add".to_string())),
                    arguments: vec![Expr::Number(1.0),
                        Expr::Number(2.0)],

                }),
            ],
        };
        assert_eq!(program.body.len(),2);
    }
}
