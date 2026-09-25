use crate::ast::{BinaryOperator, Expr};
use crate::lexer::Token;

/// Parses one complete expression from a token stream.
pub fn parse(tokens: &[Token]) -> Result<Expr, String> {
    let mut parser = Parser::new(tokens);
    let expression = parser.parse_expression(0)?;

    if let Some(token) = parser.peek() {
        return Err(format!("Unexpected token after expression: {token:?}"));
    }

    Ok(expression)
}

struct Parser<'tokens> {
    tokens: &'tokens [Token],
    position: usize,
}

impl<'tokens> Parser<'tokens> {
    fn new(tokens: &'tokens [Token]) -> Self {
        Self {
            tokens,
            position: 0,
        }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }

    fn advance(&mut self) -> Option<&Token> {
        let token = self.tokens.get(self.position);
        self.position += usize::from(token.is_some());
        token
    }

    fn parse_expression(&mut self, minimum_precedence: u8) -> Result<Expr, String> {
        let mut left = self.parse_primary()?;

        while let Some(operator) = self.peek().and_then(Self::binary_operator) {
            if operator.precedence() < minimum_precedence {
                break;
            }

            self.advance();
            let right = self.parse_expression(operator.precedence() + 1)?;
            left = Expr::Binary {
                left: Box::new(left),
                operator,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    // Future primary expressions such as identifiers, literals, and function
    // calls belong here. Binary operator parsing should not need to change.
    fn parse_primary(&mut self) -> Result<Expr, String> {
        match self.advance() {
            Some(Token::Number(number)) => Ok(Expr::Number(*number)),
            Some(token) => Err(format!("Expected an expression, found {token:?}")),
            None => Err(String::from("Expected an expression")),
        }
    }

    fn binary_operator(token: &Token) -> Option<BinaryOperator> {
        match token {
            Token::Plus => Some(BinaryOperator::Add),
            Token::Subtract => Some(BinaryOperator::Subtract),
            Token::Multiply => Some(BinaryOperator::Multiply),
            Token::Divide => Some(BinaryOperator::Divide),
            Token::Number(_) => None,
        }
    }
}

impl BinaryOperator {
    fn precedence(self) -> u8 {
        match self {
            Self::LogicalOperator => unreachable!(),
            Self::Add | Self::Subtract => 1,
            Self::Multiply | Self::Divide | Self::Remainder => 2,
            Self::Less
            | Self::LessEqual
            | Self::Greater
            | Self::GreaterEqual
            | Self::Equal
            | Self::NotEqual
            | Self::StrictEqual
            | Self::StrictNotEqual => 3,
        }
    }
}
