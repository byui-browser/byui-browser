use js::{lexer, parser, runtime};

fn main() {
    let source_code = r#"
        let total = 10;
        {
            let adjustment = 5;
            let other_adjustment = 10;
            total = (total + adjustment * 2) / other_adjustment;
        }
        total;
    "#;
    println!("Running our JS Engine!");
    println!("Source program:\n{source_code}");

    // Step 1: Lexing (String -> Tokens)
    let tokens = lexer::tokenize(source_code);
    println!("Generated Tokens: {tokens:#?}");

    // Step 2: Parsing (Tokens -> Program AST)
    match parser::parse_program(&tokens) {
        Ok(program) => {
            println!("Generated AST Tree: {program:#?}");

            // Step 3: Tree-walk evaluation (AST -> Final Value)
            match runtime::evaluate_program(&program) {
                Ok(result) => println!("Final Evaluated Result: {result:?}"),
                Err(error) => println!("Runtime Error: {error}"),
            }

            // The same path used by callers evaluating source directly.
            match js::eval(source_code) {
                Ok(result) => println!("Source Evaluation Result: {result:?}"),
                Err(error) => println!("Source Evaluation Error: {error}"),
            }
        }
        Err(error) => println!("Parser Error: {error}"),
    }
}
