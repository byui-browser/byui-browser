use js::ast::{BinaryOperator, Expr, Program, Statement, VarKind};
use js::lexer::Token;
use js::parser;

#[test]
fn constructs_program_with_variable_declaration() {
    let statement = Statement::VariableDeclaration {
        kind: VarKind::Let,
        name: "count".to_string(),
        init: Some(Expr::Binary {
            left: Box::new(Expr::Number(1.0)),
            operator: BinaryOperator::Add,
            right: Box::new(Expr::Number(2.0)),
        }),
    };

    assert_eq!(
        Program {
            body: vec![statement.clone()],
        },
        Program {
            body: vec![statement],
        }
    );
}

#[test]
fn constructs_if_statement_with_assignment_block() {
    let statement = Statement::If {
        condition: Expr::Binary {
            left: Box::new(Expr::Identifier("x".to_string())),
            operator: BinaryOperator::Greater,
            right: Box::new(Expr::Number(5.0)),
        },
        then_branch: Box::new(Statement::Block(vec![Statement::Expression(
            Expr::Assign {
                name: "x".to_string(),
                value: Box::new(Expr::Number(0.0)),
            },
        )])),
        else_branch: None,
    };

    assert!(matches!(statement, Statement::If { .. }));
}

#[test]
fn constructs_function_and_call_statements() {
    let program = Program {
        body: vec![
            Statement::FunctionDeclaration {
                name: "add".to_string(),
                params: vec!["a".to_string(), "b".to_string()],
                body: vec![Statement::Return(Some(Expr::Binary {
                    left: Box::new(Expr::Identifier("a".to_string())),
                    operator: BinaryOperator::Add,
                    right: Box::new(Expr::Identifier("b".to_string())),
                }))],
            },
            Statement::Expression(Expr::Call {
                callee: Box::new(Expr::Identifier("add".to_string())),
                arguments: vec![Expr::Number(1.0), Expr::Number(2.0)],
            }),
        ],
    };

    assert_eq!(program.body.len(), 2);
}

#[test]
fn parses_unary_negation() {
    let expression = parser::parse(&[Token::Subtract, Token::Number(7.0)]).unwrap();

    assert_eq!(
        expression,
        Expr::Unary {
            operator: js::ast::UnaryOperator::Negate,
            operand: Box::new(Expr::Number(7.0)),
        }
    );
}

#[test]
fn unary_negation_binds_tighter_than_binary_operators() {
    let expression = parser::parse(&[
        Token::Number(1.0),
        Token::Subtract,
        Token::Subtract,
        Token::Number(2.0),
    ])
    .unwrap();

    assert_eq!(
        expression,
        Expr::Binary {
            left: Box::new(Expr::Number(1.0)),
            operator: BinaryOperator::Subtract,
            right: Box::new(Expr::Unary {
                operator: js::ast::UnaryOperator::Negate,
                operand: Box::new(Expr::Number(2.0)),
            }),
        }
    );
}

#[test]
fn parses_binary_expression_and_preserves_ast_shape() {
    let expression = parser::parse(&[Token::Number(1.0), Token::Plus, Token::Number(2.0)]).unwrap();

    assert_eq!(
        expression,
        Expr::Binary {
            left: Box::new(Expr::Number(1.0)),
            operator: BinaryOperator::Add,
            right: Box::new(Expr::Number(2.0)),
        }
    );
}
