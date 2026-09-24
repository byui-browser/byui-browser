//! JavaScript engine: parser, AST, bytecode compiler, interpreter/VM, GC.
//!
//! **Owning team**: JavaScript Engine Team
//!
//! Owns the VM and values. DOM bindings live in `webapis`, not here.

#![forbid(unsafe_code)]

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
    /// The full parser and VM are still future work. This deliberately supports
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

/// A parsed program. The full statement shape is still under development.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Program {
    pub statements: Vec<Statement>,
}

/// One statement in the AST.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Statement {}

/// Parses source text into a [`Program`].
pub fn parse(source: &str) -> JsResult<Program> {
    if source.trim().is_empty() {
        return Ok(Program::default());
    }
    Err(JsError::new("JavaScript parsing is not implemented"))
}

/// Parses and evaluates source text, returning the completion value.
pub fn eval(source: &str) -> JsResult<Value> {
    let program = parse(source)?;
    if program.statements.is_empty() {
        return Ok(Value::Undefined);
    }
    Err(JsError::new("JavaScript evaluation is not implemented"))
}

#[cfg(test)]
mod tests {
    use super::{JsError, Realm, Value};
    use std::sync::Arc;

    #[test]
    fn registered_host_function_can_be_called_from_script() {
        let mut realm = Realm::new();
        realm
            .register_global_function(
                "print",
                Arc::new(|arguments| {
                    assert!(arguments.is_empty());
                    Ok(Value::Undefined)
                }),
            )
            .unwrap();

        assert_eq!(realm.evaluate_script("print()"), Ok(Value::Undefined));
    }

    #[test]
    fn unknown_global_is_an_error() {
        let realm = Realm::new();

        assert_eq!(
            realm.evaluate_script("print()"),
            Err(JsError::new("global function `print` is not defined"))
        );
    }

    #[test]
    fn empty_global_names_are_rejected() {
        let mut realm = Realm::new();

        assert_eq!(
            realm.register_global_function("", Arc::new(|_| Ok(Value::Undefined))),
            Err(JsError::new("global function name cannot be empty"))
        );
    }

    #[test]
    fn empty_program_evaluates_to_undefined() {
        assert_eq!(super::eval(""), Ok(Value::Undefined));
    }
}
