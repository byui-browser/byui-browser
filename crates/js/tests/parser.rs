use js::ast::{
    BinaryOperator as B, Expr, LogicalOperator as L, Statement as S, UnaryOperator as U, VarKind,
};
use js::{Program, parse};

fn id(name: &str) -> Expr {
    Expr::Identifier(name.into())
}
fn binary(left: Expr, operator: B, right: Expr) -> Expr {
    Expr::Binary {
        left: Box::new(left),
        operator,
        right: Box::new(right),
    }
}
fn logical(left: Expr, operator: L, right: Expr) -> Expr {
    Expr::Logical {
        left: Box::new(left),
        operator,
        right: Box::new(right),
    }
}
fn expression(source: &str) -> Expr {
    let program = parse(source).unwrap();
    assert_eq!(program.body.len(), 1);
    match program.body.into_iter().next().unwrap() {
        S::Expression(expr) => expr,
        other => panic!("Expected expression, got {other:?}"),
    }
}

#[test]
fn parses_representative_complete_program() {
    let source = "var count; let x = 1 + 2; const limit = 5;
        function add(a, b) { return a + b; }
        while (x < limit) { x = add(x, 1); }
        if (x >= limit) { count = x; } else count = 0;";
    assert_eq!(
        parse(source).unwrap(),
        Program {
            body: vec![
                S::VariableDeclaration {
                    kind: VarKind::Var,
                    name: "count".into(),
                    init: None
                },
                S::VariableDeclaration {
                    kind: VarKind::Let,
                    name: "x".into(),
                    init: Some(binary(Expr::Number(1.0), B::Add, Expr::Number(2.0)))
                },
                S::VariableDeclaration {
                    kind: VarKind::Const,
                    name: "limit".into(),
                    init: Some(Expr::Number(5.0))
                },
                S::FunctionDeclaration {
                    name: "add".into(),
                    params: vec!["a".into(), "b".into()],
                    body: vec![S::Return(Some(binary(id("a"), B::Add, id("b"))))]
                },
                S::While {
                    condition: binary(id("x"), B::Less, id("limit")),
                    body: Box::new(S::Block(vec![S::Expression(Expr::Assign {
                        name: "x".into(),
                        value: Box::new(Expr::Call {
                            callee: Box::new(id("add")),
                            arguments: vec![id("x"), Expr::Number(1.0)]
                        })
                    })]))
                },
                S::If {
                    condition: binary(id("x"), B::GreaterEqual, id("limit")),
                    then_branch: Box::new(S::Block(vec![S::Expression(Expr::Assign {
                        name: "count".into(),
                        value: Box::new(id("x"))
                    })])),
                    else_branch: Some(Box::new(S::Expression(Expr::Assign {
                        name: "count".into(),
                        value: Box::new(Expr::Number(0.0))
                    })))
                },
            ]
        }
    );
}

#[test]
fn preserves_precedence_and_logical_short_circuit_structure() {
    assert_eq!(
        expression("a || b && c == d < e + f * g"),
        logical(
            id("a"),
            L::Or,
            logical(
                id("b"),
                L::And,
                binary(
                    id("c"),
                    B::Equal,
                    binary(
                        id("d"),
                        B::Less,
                        binary(id("e"), B::Add, binary(id("f"), B::Multiply, id("g")))
                    )
                )
            )
        )
    );
    assert_eq!(
        expression("(1 + 2) * 3"),
        binary(
            binary(Expr::Number(1.0), B::Add, Expr::Number(2.0)),
            B::Multiply,
            Expr::Number(3.0)
        )
    );
    assert_eq!(
        expression("8 - 4 - 2"),
        binary(
            binary(Expr::Number(8.0), B::Subtract, Expr::Number(4.0)),
            B::Subtract,
            Expr::Number(2.0)
        )
    );
    assert_eq!(
        expression("1 - -2 / 4"),
        binary(
            Expr::Number(1.0),
            B::Subtract,
            binary(
                Expr::Unary {
                    operator: U::Negate,
                    operand: Box::new(Expr::Number(2.0))
                },
                B::Divide,
                Expr::Number(4.0)
            )
        )
    );
}

#[test]
fn assignments_are_right_associative_and_calls_bind_before_unary() {
    assert_eq!(
        expression("a = b = !factory()(c = 2, -3)"),
        Expr::Assign {
            name: "a".into(),
            value: Box::new(Expr::Assign {
                name: "b".into(),
                value: Box::new(Expr::Unary {
                    operator: U::Not,
                    operand: Box::new(Expr::Call {
                        callee: Box::new(Expr::Call {
                            callee: Box::new(id("factory")),
                            arguments: vec![]
                        }),
                        arguments: vec![
                            Expr::Assign {
                                name: "c".into(),
                                value: Box::new(Expr::Number(2.0))
                            },
                            Expr::Unary {
                                operator: U::Negate,
                                operand: Box::new(Expr::Number(3.0))
                            }
                        ],
                    }),
                })
            }),
        }
    );
}

#[test]
fn parses_all_supported_comparisons() {
    for (symbol, operator) in [
        ("<", B::Less),
        ("<=", B::LessEqual),
        (">", B::Greater),
        (">=", B::GreaterEqual),
        ("==", B::Equal),
        ("!=", B::NotEqual),
        ("===", B::StrictEqual),
        ("!==", B::StrictNotEqual),
    ] {
        assert_eq!(
            expression(&format!("a {symbol} b")),
            binary(id("a"), operator, id("b"))
        );
    }
}

#[test]
fn parses_literals_and_empty_programs() {
    assert_eq!(parse(" \n\t").unwrap(), Program::default());
    assert_eq!(
        parse("'hi\\n'; true; false; null; undefined; 1.5e2")
            .unwrap()
            .body,
        vec![
            S::Expression(Expr::String("hi\n".into())),
            S::Expression(Expr::Boolean(true)),
            S::Expression(Expr::Boolean(false)),
            S::Expression(Expr::Null),
            S::Expression(Expr::Undefined),
            S::Expression(Expr::Number(150.0))
        ]
    );
    assert_eq!(parse("{}").unwrap().body, vec![S::Block(vec![])]);
}

#[test]
fn else_binds_to_nearest_if() {
    let program = parse("if (a) if (b) x(); else y();").unwrap();
    let S::If {
        then_branch,
        else_branch,
        ..
    } = &program.body[0]
    else {
        panic!()
    };
    assert!(else_branch.is_none());
    assert!(matches!(
        then_branch.as_ref(),
        S::If {
            else_branch: Some(_),
            ..
        }
    ));
}

#[test]
fn follows_documented_semicolon_policy() {
    for source in [
        "let x",
        "{ let x = 1 }",
        "function f() { return }",
        "if (x) { x() } else { y() }",
        "function f() { return; return\n1 }",
        "function f() {} f()",
    ] {
        assert!(parse(source).is_ok(), "{source}");
    }
    assert_eq!(
        parse("function f() { return\n1 }").unwrap().body,
        vec![S::FunctionDeclaration {
            name: "f".into(),
            params: vec![],
            body: vec![S::Return(Some(Expr::Number(1.0)))]
        }]
    );
    for source in [
        "a\nb",
        "let a = 1\nlet b = 2",
        "if (x) x() else y()",
        ";",
        "a;;",
    ] {
        assert!(parse(source).is_err(), "{source}");
    }
}

#[test]
fn malformed_sources_return_errors_without_panicking() {
    for source in [
        "let = ;",
        "const x;",
        "let x =",
        "let a, b;",
        "{",
        "}",
        "if",
        "if () {}",
        "if (x {}",
        "else {}",
        "while (true)",
        "function () {}",
        "function f(a,) {}",
        "function f(a b) {}",
        "function f()",
        "return 1;",
        "1 = 2",
        "f() = 1",
        "a + b = 1",
        "f(,)",
        "f(1,)",
        "f(1",
        "(1",
        "()",
        "1 +",
        "!",
        "@",
        "'unfinished",
        "1.2.3",
        "a % b",
    ] {
        let error = parse(source).expect_err(source);
        assert!(!error.message.is_empty());
        assert!(!error.context.is_empty());
        assert!(error.offset.is_some());
    }
}

#[test]
fn diagnostics_include_source_location_context_and_eof() {
    let error = parse("let x = 'é';\nlet = 2;").unwrap_err();
    assert_eq!(error.token_index, 6);
    assert_eq!(error.offset, Some(18));
    assert_eq!((error.line, error.column), (Some(2), Some(5)));
    assert_eq!(error.context, "variable declaration");
    assert_eq!(error.found.as_deref(), Some("Assign"));
    assert!(error.to_string().contains("line 2, column 5"));
    let error = parse("f(1").unwrap_err();
    assert_eq!(error.offset, Some(3));
    assert_eq!(error.found, None);
    assert_eq!(error.context, "call arguments");
}

#[test]
fn excessive_recursive_nesting_returns_an_error() {
    for source in [
        format!("{}1{}", "(".repeat(300), ")".repeat(300)),
        format!("{}1", "!".repeat(300)),
        format!("{}{}", "{".repeat(300), "}".repeat(300)),
        format!("{}1", "a=".repeat(300)),
    ] {
        assert!(parse(&source).unwrap_err().message.contains("nesting"));
    }
}

#[test]
fn every_prefix_of_a_program_can_be_parsed_without_panicking() {
    let source = "function f(a) { while (a > 0) { a = a - 1; } return a; } let x = f(2);";
    for end in 0..=source.len() {
        let _ = parse(&source[..end]);
    }
}
