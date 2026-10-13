//! Parsing for the supported JavaScript statement and expression subset.

use crate::ast::{Expr, Program};
use crate::lexer::Token;
use crate::stack::with_engine_stack;
use std::fmt;

mod expressions;
mod statements;

/// A syntax error with token location and grammar context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// Zero-based token index; the token count denotes end of input.
    pub token_index: usize,
    /// Human-readable description of the expected syntax.
    pub message: String,
    /// Grammar construct being parsed when the error occurred.
    pub context: &'static str,
    /// Debug representation of the offending token, or `None` at end of input.
    pub found: Option<String>,
    /// UTF-8 byte offset, populated by the source-level [`crate::parse`] API.
    pub offset: Option<usize>,
    /// One-based source line, populated by [`crate::parse`].
    pub line: Option<usize>,
    /// One-based character column, populated by [`crate::parse`].
    pub column: Option<usize>,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} in {} at token {}",
            self.message, self.context, self.token_index
        )?;
        if let (Some(line), Some(column)) = (self.line, self.column) {
            write!(f, " (line {line}, column {column})")?;
        }
        write!(
            f,
            "; found {}",
            self.found.as_deref().unwrap_or("end of input")
        )
    }
}

impl std::error::Error for ParseError {}

type ParseResult<T> = Result<T, ParseError>;

/// Parses a complete token stream into an owned program AST.
///
/// Simple statements require `;`, except immediately before `}` or end of
/// input. Newlines are whitespace, not automatic semicolon insertion; in
/// particular `return` followed by a newline may still have a value. Empty
/// statements, comma declarations, and trailing commas are unsupported.
/// Returns outside functions, constants without initializers, duplicate
/// parameter names, and `let`/`const`/`function` declarations used directly as
/// an `if`, `else`, or `while` body are rejected.
///
/// To bound stack usage, nesting beyond 128 recursive grammar levels and any
/// expression tree deeper than 512 nodes (including long left-associative
/// chains such as `1 + 1 + ...`) return an error.
pub fn parse_program(tokens: &[Token]) -> ParseResult<Program> {
    with_engine_stack(|| {
        let mut parser = Parser::new(tokens);
        let mut body = Vec::new();
        while parser.peek().is_some() {
            body.push(parser.parse_statement()?);
        }
        Ok(Program { body })
    })
}

/// Parses one complete expression into an owned AST.
///
/// Uses the same expression grammar and nesting bound as [`parse_program`].
/// Trailing tokens, including semicolons, are rejected. Errors include a token
/// index; source positions are supplied by the program-level [`crate::parse`].
pub fn parse(tokens: &[Token]) -> ParseResult<Expr> {
    with_engine_stack(|| {
        let mut parser = Parser::new(tokens);
        let expression = parser.parse_expression(0)?;
        if parser.peek().is_some() {
            return Err(parser.error("Unexpected token after expression", "expression"));
        }
        Ok(expression)
    })
}

/// Maximum number of recursive grammar levels.
const MAX_NESTING: usize = 128;

/// Maximum depth of one expression tree. Recursion depth alone does not bound
/// this, because binary chains and call suffixes are built in loops.
const MAX_EXPRESSION_DEPTH: usize = 512;

const NESTING_ERROR: &str = "Maximum parser nesting exceeded";

struct Parser<'tokens> {
    tokens: &'tokens [Token],
    position: usize,
    depth: usize,
    function_depth: usize,
}

impl<'tokens> Parser<'tokens> {
    fn new(tokens: &'tokens [Token]) -> Self {
        Self {
            tokens,
            position: 0,
            depth: 0,
            function_depth: 0,
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

    fn error(&self, message: impl Into<String>, context: &'static str) -> ParseError {
        ParseError {
            token_index: self.position,
            message: message.into(),
            context,
            found: self.peek().map(|token| format!("{token:?}")),
            offset: None,
            line: None,
            column: None,
        }
    }

    fn consume(&mut self, token: &Token) -> bool {
        if self.peek() == Some(token) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, token: &Token, context: &'static str) -> ParseResult<()> {
        if self.consume(token) {
            Ok(())
        } else {
            Err(self.error(format!("Expected {token:?}"), context))
        }
    }

    fn identifier(&mut self, context: &'static str) -> ParseResult<String> {
        match self.peek() {
            Some(Token::Identifier(name)) => {
                let name = name.clone();
                self.advance();
                Ok(name)
            }
            _ => Err(self.error("Expected an identifier", context)),
        }
    }

    fn nested<T>(
        &mut self,
        context: &'static str,
        parse: impl FnOnce(&mut Self) -> ParseResult<T>,
    ) -> ParseResult<T> {
        if self.depth >= MAX_NESTING {
            return Err(self.error(NESTING_ERROR, context));
        }
        self.depth += 1;
        let result = parse(self);
        self.depth -= 1;
        result
    }

    fn parse_expression(&mut self, minimum_precedence: u8) -> ParseResult<Expr> {
        self.parse_expression_with_depth(minimum_precedence)
            .map(|(expression, _)| expression)
    }

    /// Parses an expression and reports the depth of its tree.
    fn parse_expression_with_depth(
        &mut self,
        minimum_precedence: u8,
    ) -> ParseResult<(Expr, usize)> {
        self.nested("expression", |parser| {
            expressions::parse(parser, minimum_precedence)
        })
    }

    /// Validates the depth of a node about to be built from its children.
    fn node_depth(&self, depth: usize, context: &'static str) -> ParseResult<usize> {
        if depth > MAX_EXPRESSION_DEPTH {
            Err(self.error(NESTING_ERROR, context))
        } else {
            Ok(depth)
        }
    }

    fn parse_statement(&mut self) -> ParseResult<crate::ast::Statement> {
        self.nested("statement", statements::parse)
    }

    fn terminator(&mut self, context: &'static str) -> ParseResult<()> {
        if self.consume(&Token::Semicolon) || matches!(self.peek(), None | Some(Token::RightBrace))
        {
            Ok(())
        } else {
            Err(self.error("Expected a semicolon", context))
        }
    }
}
