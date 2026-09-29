pub fn tokenize(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();

    while let Some(&c) = chars.peek() {
        match c {
            ' ' | '\t' | '\n' => { chars.next(); } // skip whitespace, don't split on it
            '+' => { tokens.push(Token::Plus); chars.next(); }
            '-' => { tokens.push(Token::Subtract); chars.next(); }
            '*' => { tokens.push(Token::Multiply); chars.next(); }
            '/' => { tokens.push(Token::Divide); chars.next(); }
            c if c.is_ascii_digit() => tokens.push(scan_number(&mut chars)),
            c if c.is_ascii_alphabetic() || c == '_' || c == '$' => {
                tokens.push(scan_identifier_or_keyword(&mut chars))
            }
            _ => {
                // don't silently drop this — you have no idea it's a bug otherwise
                panic!("unexpected character: {c}");
            }
        }
    }
    tokens
}