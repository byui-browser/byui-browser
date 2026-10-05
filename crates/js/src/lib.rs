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

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

/// Values that can cross the host-function boundary.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Undefined,
    Null,
    Boolean(bool),
    Number(f64),
    String(String),
}

/// Errors raised while resolving or invoking JavaScript host functions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsError {
    pub message: String,
}

impl JsError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for JsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for JsError {}

pub type JsResult<T> = Result<T, JsError>;

/// A Rust function that can be exposed as a JavaScript global.
pub type HostFunction = Arc<dyn Fn(&[Value]) -> JsResult<Value> + Send + Sync>;

/// A JavaScript execution realm with a global host-function table.
#[derive(Default)]
pub struct Realm {
    globals: HashMap<String, HostFunction>,
}

impl Realm {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a host function under a global JavaScript name.
    pub fn register_global_function(
        &mut self,
        name: impl Into<String>,
        function: HostFunction,
    ) -> JsResult<()> {
        let name = name.into();
        if name.is_empty() {
            return Err(JsError::new("global function name cannot be empty"));
        }

        self.globals.insert(name, function);
        Ok(())
    }

    /// Calls a registered global function.
    pub fn call_global(&self, name: &str, arguments: &[Value]) -> JsResult<Value> {
        let function = self
            .globals
            .get(name)
            .ok_or_else(|| JsError::new(format!("global function `{name}` is not defined")))?;
        function(arguments)
    }

    /// Evaluates the minimal call expression needed by the first Web API slice.
    ///
    /// The full VM is still future work. This deliberately supports
    /// only a global identifier followed by an empty argument list, such as
    /// `print()`, so the host-function path can be exercised end to end.
    pub fn evaluate_script(&self, source: &str) -> JsResult<Value> {
        let expression = source.trim().trim_end_matches(';').trim();
        let call = expression
            .strip_suffix(')')
            .and_then(|prefix| prefix.strip_suffix('('))
            .ok_or_else(|| JsError::new("expected a function call expression"))?;
        let name = call.trim();

        if name.is_empty() || name.contains(char::is_whitespace) {
            return Err(JsError::new("expected a global function name"));
        }

        self.call_global(name, &[])
    }
}

/// The owned statement and expression tree produced by parsing.
pub use ast::{Program, Statement};

/// Parses source text into an owned [`Program`].
///
/// Uses the subset and semicolon policy documented by [`parser::parse_program`].
/// Syntax errors carry a token index, UTF-8 byte offset, and one-based line and
/// character column. Locations at end of input point just past the source.
pub fn parse(source: &str) -> Result<Program, parser::ParseError> {
    let entries = lexer::tokenize_spanned(source);
    let starts: Vec<_> = entries.iter().map(|entry| entry.start).collect();
    let tokens: Vec<_> = entries.into_iter().map(|entry| entry.token).collect();
    parser::parse_program(&tokens).map_err(|mut error| {
        let offset = starts
            .get(error.token_index)
            .copied()
            .unwrap_or(source.len());
        let prefix = &source[..offset];
        error.offset = Some(offset);
        error.line = Some(prefix.chars().filter(|ch| *ch == '\n').count() + 1);
        error.column = Some(prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1);
        error
    })
}

/// Parses and evaluates source text, returning the completion value.
///
/// Supports the existing tree-walk runtime: expressions, declarations, and
/// blocks. Other parsed statement forms and calls return runtime errors.
pub fn eval(source: &str) -> JsResult<Value> {
    let program = parse(source).map_err(|error| JsError::new(error.to_string()))?;
    runtime::evaluate_program(&program)
}
