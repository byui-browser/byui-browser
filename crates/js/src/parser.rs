// Bring our Token enum from the lexer file into scope
use crate::lexer::Token;

// Define our Abstract Syntax Tree (AST) node types

#[derive(Debug, PartialEq)]
pub enum Expr {
    Number(f64),
    // An addition expression holding a left branch and a right branch
    Add(Box<Expr>, Box<Expr>),
}

// A function to parse a list of tokens into an AST expression

pub fn parse(tokens: &[Token]) -> Result<Expr, String> {
    // For our current test case: "42 + 10"
    // We expect exactly 3 tokens: [Number, Plus, Number]

    if tokens.len() == 3 {
        if let(Token::Number(left), Token::Plus, Token::Number(right)) = (&tokens[0], &tokens[1], &tokens[2]) {
            return Ok(Expr::Add(
                Box::new(Expr::Number(*left)),
                Box::new(Expr::Number(*right))
            ));
        }
    }
    Err(String::from("Failed to parse expression"))
}

// A function to evaluate our AST tree into a final number

pub fn eval(expr: &Expr) -> f64 {
    match expr {
        Expr::Number(n) => *n,
        Expr::Add(left,right) => eval(left) + eval(right),
    }
}

