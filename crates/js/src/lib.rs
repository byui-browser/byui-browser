//! JavaScript engine: parser, AST, bytecode compiler, interpreter/VM, GC.
//!
//! **Owning team**: JavaScript Engine Team
//!
//! Owns the VM and values. DOM bindings live in `webapis`, not here.

#![forbid(unsafe_code)]

pub mod ast;
pub mod lexer;
pub mod parser;
pub mod runtime;

use std::fmt;

/// A JavaScript runtime value.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Reshape the
// types, names, and module layout however your crate's public API needs.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Undefined,
    Null,
    Boolean(bool),
    Number(f64),
    String(String),
}

/// A parsed program. Statement shape is the JS team's call.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Program {
    pub statements: Vec<Statement>,
}

/// One statement in the AST. Intentionally empty until the parser exists.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Statement {}

/// Parse or runtime failure.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsError {
    pub message: String,
}

impl fmt::Display for JsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for JsError {}

/// Parses source text into a [`Program`].
///
/// Currently only handles empty or whitespace-only source.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Reshape the
// signature however your crate's public API needs.
pub fn parse(source: &str) -> Result<Program, JsError> {
    if source.trim().is_empty() {
        return Ok(Program::default());
    }
    todo!("TODO(js): parse: {source:?}")
}

/// Parses and evaluates source text, returning the completion value.
///
/// Currently only handles empty or whitespace-only source.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Reshape the
// signature however your crate's public API needs.
pub fn eval(source: &str) -> Result<Value, JsError> {
    let program = parse(source)?;
    if program.statements.is_empty() {
        return Ok(Value::Undefined);
    }
    todo!("TODO(js): evaluate {} statements", program.statements.len())
}
