#[derive(Debug, PartialEq)]
pub enum Token {
    Number(f64),
    Plus,
}

pub fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let bytes = input.as_bytes();
    let mut tokens = Vec::with_capacity(bytes.len() / 2 + 1); // rough guess, avoids regrowing
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,                // skip whitespace
            b'+' => { tokens.push(Token::Plus); i += 1; }
            b'0'..=b'9' | b'.' => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                    i += 1;
                }
                let text = &input[start..i];                      // slice, no allocation
                let n = text.parse::<f64>()
                    .map_err(|_| format!("invalid number '{}' at position {}", text, start))?;
                tokens.push(Token::Number(n));
            }
            other => return Err(format!("unexpected character '{}' at position {}", other as char, i)),
        }
    }
    Ok(tokens)
}