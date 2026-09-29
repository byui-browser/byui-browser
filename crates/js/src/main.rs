use js::{lexer, parser, runtime};

fn main() {
    let source_code = "42 + 10";
    println!("Running our JS Engine!");

    // Step 1: Lexing (String -> Tokens)
    let tokens = lexer::tokenize(source_code);

    // Step 2: Parsing (Tokens -> AST)
    match parser::parse(&tokens) {
        Ok(ast) => {
            let result = runtime::evaluate(&ast);
            println!("Generated AST Tree: {:#?}", ast);

            // Step 3: Evaluation (AST -> Final Number Result)

            println!("Final Evaluated Result: {:?}", result);
        }
        Err(e) => println!("Parser Error: {}", e),
    }
}
