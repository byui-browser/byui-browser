//! JavaScript engine: parser, AST, bytecode compiler, interpreter/VM, GC.
//!
//! **Owning team**: JavaScript Engine Team
//!
//! Owns the VM and values. DOM bindings live in `webapis`, not here.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod ast;
pub mod lexer;
pub mod parser;
pub mod runtime;
mod stack;

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

pub use runtime::{Function, Object};

/// A JavaScript value produced by evaluation or passed to host functions.
///
/// Primitive variants own their data. [`Value::Function`] and
/// [`Value::Object`] are reference handles: cloning the value shares the same
/// function or property table, and `==`/`===` compare them by identity.
///
/// Unsupported: symbols, bigints, arrays, and other built-in object kinds.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// The `undefined` value.
    Undefined,
    /// The `null` value.
    Null,
    /// A boolean.
    Boolean(bool),
    /// An IEEE 754 double-precision number, as used by all JS numbers.
    Number(f64),
    /// An owned string.
    String(String),
    /// A callable function, compared by identity.
    Function(Function),
    /// A plain object with string-keyed properties, compared by identity.
    /// See [`Object`] for its sharing rules and limitations.
    Object(Object),
}

/// Formats the value as JavaScript's `ToString` would, so numbers print as
/// `1`, `1e+21`, or `Infinity` rather than using Rust float formatting.
impl fmt::Display for Value {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Undefined => formatter.write_str("undefined"),
            Self::Null => formatter.write_str("null"),
            Self::Boolean(value) => write!(formatter, "{value}"),
            Self::Number(number) => formatter.write_str(&runtime::number_to_string(*number)),
            Self::String(text) => formatter.write_str(text),
            Self::Function(function) => write!(formatter, "{function}"),
            Self::Object(object) => write!(formatter, "{object}"),
        }
    }
}

/// The broad category of a JavaScript engine error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsErrorCategory {
    /// The source could not be parsed.
    Syntax,
    /// The program performed an invalid operation.
    Runtime,
    /// The program exceeded a configured execution limit.
    Limit,
    /// A registered host function reported a failure.
    Host,
}

/// Errors raised while parsing, evaluating, or invoking host functions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsError {
    /// The broad category of the failure.
    pub category: JsErrorCategory,
    /// Human-readable description of the failure.
    pub message: String,
    /// The interpreter operation that failed, when known.
    pub context: Option<String>,
}

impl JsError {
    /// Creates an error carrying `message`.
    pub fn new(message: impl Into<String>) -> Self {
        Self::runtime(message)
    }

    /// Creates a runtime error carrying `message`.
    pub fn runtime(message: impl Into<String>) -> Self {
        Self {
            category: JsErrorCategory::Runtime,
            message: message.into(),
            context: None,
        }
    }

    /// Creates a categorized error with an interpreter operation as context.
    pub fn with_context(
        category: JsErrorCategory,
        message: impl Into<String>,
        context: impl Into<String>,
    ) -> Self {
        Self {
            category,
            message: message.into(),
            context: Some(context.into()),
        }
    }
}

impl fmt::Display for JsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for JsError {}

/// Result type used throughout the engine, failing with [`JsError`].
pub type JsResult<T> = Result<T, JsError>;

/// A Rust function that can be exposed as a JavaScript global.
pub type HostFunction = Arc<dyn Fn(&[Value]) -> JsResult<Value> + Send + Sync>;

/// A JavaScript execution realm with a table of host-provided globals.
///
/// Globals are installed in a scope enclosing every script, so a script can
/// shadow them with its own declarations, and assigning to a global name
/// changes it for that script only. Objects registered as globals are shared
/// handles, however: property changes a script makes to them persist across
/// [`Realm::evaluate_script`] calls and are visible to the host.
pub struct Realm {
    functions: HashMap<String, HostFunction>,
    values: HashMap<String, Value>,
    limits: runtime::ExecutionLimits,
}

impl Default for Realm {
    fn default() -> Self {
        Self::new()
    }
}

impl Realm {
    /// Creates a realm with no registered globals.
    pub fn new() -> Self {
        Self {
            functions: HashMap::new(),
            values: HashMap::new(),
            limits: runtime::ExecutionLimits::default(),
        }
    }

    /// Registers a host function under a global JavaScript name, replacing
    /// any global previously registered under that name.
    ///
    /// The realm keeps its own reference to `function` and shares it with
    /// every script. Scripts can only reach names that are valid identifiers;
    /// a dotted name such as `localStorage.setItem` is a single global reachable
    /// only through [`Realm::call_global`]. To expose `localStorage.setItem` to
    /// scripts, register an [`Object`] named `localStorage` with a
    /// `setItem` method instead (see [`Realm::register_global_value`]).
    pub fn register_global_function(
        &mut self,
        name: impl Into<String>,
        function: HostFunction,
    ) -> JsResult<()> {
        let name = name.into();
        if name.is_empty() {
            return Err(JsError::new("global function name cannot be empty"));
        }

        self.values.remove(&name);
        self.functions.insert(name, function);
        Ok(())
    }

    /// Registers any value, such as an [`Object`] namespace like `console` or
    /// `document`, under a global JavaScript name, replacing any global
    /// previously registered under that name.
    ///
    /// The realm stores `value` and gives each script a clone of it. For an
    /// [`Object`] that clone is the same shared handle, so the host can keep
    /// its own clone to read properties scripts set, and scripts see
    /// properties the host sets between evaluations. Methods are added with
    /// [`Object::set_method`] and receive only their call arguments, not the
    /// object as `this`.
    ///
    /// ```
    /// use std::sync::Arc;
    /// use js::{Object, Realm, Value};
    ///
    /// let console = Object::new();
    /// console.set_method("log", Arc::new(|_| Ok(Value::Undefined)));
    /// let mut realm = Realm::new();
    /// realm.register_global_value("console", Value::Object(console.clone())).unwrap();
    ///
    /// realm.evaluate_script("console.log('hi'); console.level = 2;").unwrap();
    /// assert_eq!(console.get("level"), Some(Value::Number(2.0)));
    /// ```
    pub fn register_global_value(&mut self, name: impl Into<String>, value: Value) -> JsResult<()> {
        let name = name.into();
        if name.is_empty() {
            return Err(JsError::new("global value name cannot be empty"));
        }

        self.functions.remove(&name);
        self.values.insert(name, value);
        Ok(())
    }

    /// Calls a global registered with [`Realm::register_global_function`].
    /// Values registered with [`Realm::register_global_value`] are not
    /// callable through this method, even if they are functions.
    pub fn call_global(&self, name: &str, arguments: &[Value]) -> JsResult<Value> {
        let function = self
            .functions
            .get(name)
            .ok_or_else(|| JsError::new(format!("global function `{name}` is not defined")))?;
        function(arguments)
    }

    /// Replaces the limits used by future script evaluations.
    pub fn set_execution_limits(&mut self, limits: runtime::ExecutionLimits) {
        self.limits = limits;
    }

    /// Returns the limits used by this realm.
    pub fn execution_limits(&self) -> runtime::ExecutionLimits {
        self.limits
    }

    /// Parses and evaluates a script with the registered host functions in
    /// scope, returning its completion value.
    ///
    /// Supports the same language subset as [`eval`]. Each call runs in a
    /// fresh script scope: bindings declared by one script are not visible to
    /// the next.
    pub fn evaluate_script(&self, source: &str) -> JsResult<Value> {
        let program = parse(source).map_err(|error| {
            JsError::with_context(JsErrorCategory::Syntax, error.to_string(), "parsing script")
        })?;
        runtime::evaluate_program_with_globals(&program, &self.functions, &self.values, self.limits)
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
/// Runs the tree-walk interpreter described in [`runtime`]: expressions,
/// member access, declarations, blocks, `if`, `while`, functions, calls, and
/// `return`. Scripts cannot create objects themselves yet (there are no
/// object literals), so member access needs objects from a [`Realm`]. Syntax
/// errors and runtime errors are both returned as [`JsError`].
pub fn eval(source: &str) -> JsResult<Value> {
    let program = parse(source).map_err(|error| {
        JsError::with_context(JsErrorCategory::Syntax, error.to_string(), "parsing script")
    })?;
    runtime::evaluate_program(&program)
}

/// Parses and evaluates source text using caller-provided execution limits.
pub fn eval_with_limits(source: &str, limits: runtime::ExecutionLimits) -> JsResult<Value> {
    let program = parse(source).map_err(|error| {
        JsError::with_context(JsErrorCategory::Syntax, error.to_string(), "parsing script")
    })?;
    runtime::evaluate_program_with_limits(&program, limits)
}
