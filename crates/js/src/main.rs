// Tell main.rs that the lexer file exists
mod lexer;
mod parser;

fn main() {
    println!("Running our JS Engine!");

    let source_code = "42 + 10";
    
    // Step 1: Lexing (String -> Tokens)
    let tokens = lexer::tokenize(source_code);

    // Step 2: Parsing (Tokens -> AST)
    match parser::parse(&tokens) {
        Ok(ast) => {
            println!("Generated AST Tree: {:#?}", ast);
            
            // Step 3: Evaluation (AST -> Final Number Result)
            let result = parser::eval(&ast);
            println!("Final Evaluated Result: {}", result);
        }
        Err(e) => println!("Parser Error: {}", e),
    }
}



