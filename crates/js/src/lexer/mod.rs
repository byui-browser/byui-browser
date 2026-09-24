// Starting out Lexer Token
// Step 1
#[derive(Debug, PartialEq)]
pub enum Token {
    Number(f64),
    Plus,
    Subtract,
    Multiply,
    Divide,
}

// 2. A function that takes a full string and returns a LIST (Vector) of Tokens
pub fn tokenize(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();

    // Split the input string wherever there is whitespace (spaces)
    for word in input.split_whitespace() {
        match word {
            "+" => tokens.push(Token::Plus),
            "-" => tokens.push(Token::Subtract),
            "*" => tokens.push(Token::Multiply),
            "/" => tokens.push(Token::Divide),
            _ => {
                // Try to parse the word as a number
                if let Ok(number_value) = word.parse::<f64>() {
                    tokens.push(Token::Number(number_value));
                }
            }
        }
    }
    tokens
}
