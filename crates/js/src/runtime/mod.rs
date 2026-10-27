//! Tree-walk interpreter for parsed programs.
//!
//! Supports expressions, `var`/`let`/`const` declarations, blocks, `if`,
//! `while`, function declarations with closures, calls, `return`, and member
//! access (`object.name`, `object[key]`, and assignment to either).
//!
//! Semantics follow ECMAScript where supported: `var` and function
//! declarations are hoisted to the enclosing function or script, `let` and
//! `const` are block scoped and unusable before their declaration runs, and
//! operators use JavaScript type conversions.
//!
//! Limitations:
//! - Assigning to an undeclared name is an error, as in strict mode.
//! - Member access works only on [`Object`] values. Reading a property of any
//!   other value, including strings and functions, is a runtime error, and so
//!   is reading a property the object does not have (JavaScript would return
//!   `undefined`). Methods are called without `this`.
//! - Classes, `new`, and `super` parse but are not evaluated here; they run
//!   only through the bytecode compiler and [`crate::vm::Vm`].
//! - There is no garbage collector yet. A function stored in the scope it
//!   closes over forms an `Arc` cycle, so its scope is never freed.
//! - Evaluation runs on a dedicated engine thread with a large stack. Deep
//!   recursion, such as a function that calls itself forever, returns a
//!   "Maximum call stack size exceeded" error once that stack is nearly full,
//!   instead of overflowing the native stack.
//! - Each evaluation has a step budget (see [`DEFAULT_STEP_LIMIT`]), so
//!   endless loops fail with an error instead of running forever.

mod conversions;
mod object;

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::ast::{
    BinaryOperator, Expr, LogicalOperator, MemberProperty, Program, Statement, UnaryOperator,
    VarKind,
};
use crate::bytecode::FunctionTemplate;
use crate::stack::{StackGuard, with_engine_stack};
use crate::{HostFunction, JsError, JsResult, Value};
use conversions::{
    is_truthy, less_than, loose_equals, strict_equals, to_int32, to_number, to_primitive, to_uint32,
};
pub(crate) use conversions::{number_to_string, to_property_key};
pub use object::Object;
use object::WeakObject;

const STACK_OVERFLOW: &str = "Maximum call stack size exceeded";

const STEP_LIMIT_EXCEEDED: &str = "Script exceeded the evaluation step limit";

/// Steps a script may take by default, where one step is evaluating one
/// statement or expression. This stops scripts such as `while (true) {}`
/// from hanging the engine thread. It is a stand-in for a time-based or
/// host-controlled interrupt, which does not exist yet.
pub const DEFAULT_STEP_LIMIT: u64 = 10_000_000;

/// Default maximum number of nested script-function calls.
pub const DEFAULT_CALL_DEPTH_LIMIT: u64 = 1_024;

/// Default maximum number of `while` iterations in one evaluation.
pub const DEFAULT_LOOP_ITERATION_LIMIT: u64 = 1_000_000;

/// Resource limits applied independently to each program evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecutionLimits {
    /// Maximum number of statements and expressions evaluated.
    pub step_limit: u64,
    /// Maximum number of nested script-function calls.
    pub call_depth_limit: u64,
    /// Maximum total number of `while` loop iterations.
    pub loop_iteration_limit: u64,
}

impl Default for ExecutionLimits {
    fn default() -> Self {
        Self {
            step_limit: DEFAULT_STEP_LIMIT,
            call_depth_limit: DEFAULT_CALL_DEPTH_LIMIT,
            loop_iteration_limit: DEFAULT_LOOP_ITERATION_LIMIT,
        }
    }
}

/// Locks a scope. No code panics while holding the lock, so a poisoned lock
/// still holds consistent bindings and is recovered rather than propagated.
trait LockScope {
    fn lock_scope(&self) -> MutexGuard<'_, Scope>;
}

impl LockScope for Mutex<Scope> {
    fn lock_scope(&self) -> MutexGuard<'_, Scope> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Debug)]
struct Binding {
    /// `None` while a `let` or `const` binding is before its declaration.
    value: Option<Value>,
    mutable: bool,
    /// Whether the binding came from `let`, `const`, or a block function.
    lexical: bool,
    /// Whether the host registered this global and no script has declared it
    /// since. A script-level `let`/`const` may shadow a host binding but not a
    /// script-declared one.
    host: bool,
}

#[derive(Default)]
struct Scope {
    bindings: HashMap<String, Binding>,
    parent: Option<ScopeRef>,
}

type ScopeRef = Arc<Mutex<Scope>>;

fn child_scope(parent: &ScopeRef) -> ScopeRef {
    Arc::new(Mutex::new(Scope {
        bindings: HashMap::new(),
        parent: Some(Arc::clone(parent)),
    }))
}

/// Applies `action` to the nearest binding named `name`.
fn with_binding<T>(
    scope: &ScopeRef,
    name: &str,
    action: impl FnOnce(&mut Binding) -> JsResult<T>,
) -> JsResult<T> {
    let mut current = Arc::clone(scope);
    loop {
        if let Some(binding) = current.lock_scope().bindings.get_mut(name) {
            return action(binding);
        }
        let parent = current.lock_scope().parent.clone();
        match parent {
            Some(parent) => current = parent,
            None => return Err(JsError::new(format!("`{name}` is not defined"))),
        }
    }
}

fn read(scope: &ScopeRef, name: &str) -> JsResult<Value> {
    with_binding(scope, name, |binding| {
        binding.value.clone().ok_or_else(|| uninitialized(name))
    })
}

fn assign(scope: &ScopeRef, name: &str, value: Value) -> JsResult<()> {
    with_binding(scope, name, |binding| {
        if binding.value.is_none() {
            return Err(uninitialized(name));
        }
        if !binding.mutable {
            return Err(JsError::new(format!("Assignment to constant `{name}`")));
        }
        binding.value = Some(value);
        Ok(())
    })
}

fn uninitialized(name: &str) -> JsError {
    JsError::new(format!("Cannot access `{name}` before initialization"))
}

fn already_declared(name: &str) -> JsError {
    JsError::new(format!("`{name}` has already been declared"))
}

/// Declares a `var`-style binding in exactly `scope`, keeping any existing
/// `var` binding of the same name.
fn declare_var(scope: &ScopeRef, name: &str) -> JsResult<()> {
    let mut scope = scope.lock_scope();
    match scope.bindings.get_mut(name) {
        Some(binding) if binding.lexical => Err(already_declared(name)),
        Some(binding) => {
            binding.host = false;
            Ok(())
        }
        None => {
            scope.bindings.insert(
                name.to_owned(),
                Binding {
                    value: Some(Value::Undefined),
                    mutable: true,
                    lexical: false,
                    host: false,
                },
            );
            Ok(())
        }
    }
}

/// Declares a block-scoped binding in exactly `scope`.
fn declare_lexical(
    scope: &ScopeRef,
    name: &str,
    mutable: bool,
    value: Option<Value>,
) -> JsResult<()> {
    let mut scope = scope.lock_scope();
    if scope.bindings.contains_key(name) {
        return Err(already_declared(name));
    }
    scope.bindings.insert(
        name.to_owned(),
        Binding {
            value,
            mutable,
            lexical: true,
            host: false,
        },
    );
    Ok(())
}

/// Sets a binding that was hoisted into exactly `scope`.
fn initialize(scope: &ScopeRef, name: &str, value: Value) {
    if let Some(binding) = scope.lock_scope().bindings.get_mut(name) {
        binding.value = Some(value);
    }
}

/// A chain of lexical scopes used while executing a JavaScript program.
pub struct Environment {
    scope: ScopeRef,
}

impl Environment {
    /// Creates an environment with one empty global scope.
    pub fn new() -> Self {
        Self {
            scope: Arc::default(),
        }
    }

    /// Reads the nearest binding with `name`.
    ///
    /// Fails if no binding exists or if a `let`/`const` binding has not been
    /// initialized yet.
    pub fn get(&self, name: &str) -> JsResult<Value> {
        read(&self.scope, name)
    }

    /// Updates the nearest mutable binding with `name`, returning `value`.
    ///
    /// Fails for undeclared names, constants, and uninitialized bindings.
    pub fn set(&mut self, name: &str, value: Value) -> JsResult<Value> {
        assign(&self.scope, name, value.clone())?;
        Ok(value)
    }
}

impl Default for Environment {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Environment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Environment")
            .finish_non_exhaustive()
    }
}

/// A callable JavaScript function value.
///
/// Cloning is cheap and preserves identity: clones compare equal with `==`
/// and `===`, while separately created functions never do.
#[derive(Clone)]
pub struct Function(Arc<FunctionKind>);

enum FunctionKind {
    Script {
        name: String,
        params: Vec<String>,
        body: Vec<Statement>,
        closure: ScopeRef,
    },
    Host {
        name: String,
        function: HostFunction,
    },
    /// A function compiled to bytecode; only the VM can run it.
    Bytecode {
        template: Arc<FunctionTemplate>,
        /// Own properties, such as a constructor's `prototype`.
        properties: Object,
        /// The prototype object that `super.name` is looked up from. It owns
        /// its methods, so the link back is weak.
        home_object: Option<WeakObject>,
        /// The parent class constructor of a derived class.
        parent: Option<Function>,
    },
}

impl Function {
    /// Wraps a Rust host function as a JavaScript function named `name`.
    pub fn from_host(name: impl Into<String>, function: HostFunction) -> Self {
        Self(Arc::new(FunctionKind::Host {
            name: name.into(),
            function,
        }))
    }

    /// The function's declared name.
    pub fn name(&self) -> &str {
        match &*self.0 {
            FunctionKind::Script { name, .. } | FunctionKind::Host { name, .. } => name,
            FunctionKind::Bytecode { template, .. } => &template.name,
        }
    }

    /// Creates a bytecode function. Constructors (plain functions and
    /// classes) get a `prototype` object, `prototype` itself when given and a
    /// fresh one otherwise, whose `constructor` refers back to the function
    /// without keeping it alive.
    pub(crate) fn from_template(
        template: Arc<FunctionTemplate>,
        home_object: Option<Object>,
        parent: Option<Function>,
        prototype: Option<Object>,
    ) -> Self {
        let constructible = template.kind.is_constructor();
        let function = Self(Arc::new(FunctionKind::Bytecode {
            template,
            properties: Object::new(),
            home_object: home_object.as_ref().map(Object::downgrade),
            parent,
        }));
        if constructible {
            let prototype = prototype.unwrap_or_default();
            prototype.set_constructor(&function);
            function.set_own("prototype", Value::Object(prototype));
        }
        function
    }

    /// The compiled template, for bytecode functions.
    pub(crate) fn template(&self) -> Option<&Arc<FunctionTemplate>> {
        match &*self.0 {
            FunctionKind::Bytecode { template, .. } => Some(template),
            _ => None,
        }
    }

    /// The prototype object `super.name` reads from, for bytecode methods.
    pub(crate) fn home_object(&self) -> Option<Object> {
        match &*self.0 {
            FunctionKind::Bytecode { home_object, .. } => {
                home_object.as_ref().and_then(WeakObject::upgrade)
            }
            _ => None,
        }
    }

    /// Returns a non-owning reference to this function.
    pub(crate) fn downgrade(&self) -> WeakFunction {
        WeakFunction(Arc::downgrade(&self.0))
    }

    /// The parent class constructor, for derived bytecode classes.
    pub(crate) fn parent(&self) -> Option<&Function> {
        match &*self.0 {
            FunctionKind::Bytecode { parent, .. } => parent.as_ref(),
            _ => None,
        }
    }

    /// Reads an own property; only bytecode functions have any.
    pub(crate) fn get_own(&self, key: &str) -> Option<Value> {
        match &*self.0 {
            FunctionKind::Bytecode { properties, .. } => properties.get_own(key),
            _ => None,
        }
    }

    /// Writes an own property, returning `false` for functions that cannot
    /// hold properties (script and host functions).
    pub(crate) fn set_own(&self, key: &str, value: Value) -> bool {
        match &*self.0 {
            FunctionKind::Bytecode { properties, .. } => {
                properties.set(key, value);
                true
            }
            _ => false,
        }
    }

    /// Calls a host function directly. Returns `None` for any other kind.
    pub(crate) fn call_host(&self, arguments: &[Value]) -> Option<JsResult<Value>> {
        match &*self.0 {
            FunctionKind::Host { function, .. } => Some(function(arguments)),
            _ => None,
        }
    }
}

/// A non-owning reference to a [`Function`], used to break reference cycles.
#[derive(Clone)]
pub(crate) struct WeakFunction(std::sync::Weak<FunctionKind>);

impl WeakFunction {
    /// Returns the function if something else still keeps it alive.
    pub(crate) fn upgrade(&self) -> Option<Function> {
        self.0.upgrade().map(Function)
    }
}

impl PartialEq for Function {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl fmt::Debug for Function {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Function({})", self.name())
    }
}

/// The JavaScript string form of the function. Source text is not retained,
/// so script functions show their signature with a placeholder body.
impl fmt::Display for Function {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &*self.0 {
            FunctionKind::Script { name, params, .. } => {
                write!(
                    formatter,
                    "function {name}({}) {{ [code] }}",
                    params.join(", ")
                )
            }
            FunctionKind::Host { name, .. } => {
                write!(formatter, "function {name}() {{ [native code] }}")
            }
            FunctionKind::Bytecode { template, .. } if template.kind.is_class() => {
                write!(formatter, "class {} {{ [code] }}", template.name)
            }
            FunctionKind::Bytecode { template, .. } => {
                write!(formatter, "function {}() {{ [code] }}", template.name)
            }
        }
    }
}

/// Evaluates a parsed program with a fresh environment, returning its
/// completion value (the value of the last value-producing statement).
///
/// Runs on the engine thread described in the module documentation and
/// stops after [`DEFAULT_STEP_LIMIT`] steps.
pub fn evaluate_program(program: &Program) -> JsResult<Value> {
    evaluate_program_with_limits(program, ExecutionLimits::default())
}

/// Evaluates a program like [`evaluate_program`], but fails with a
/// step-limit error once `step_limit` statements and expressions have been
/// evaluated. A limit of `0` rejects every non-empty program.
pub fn evaluate_program_with_step_limit(program: &Program, step_limit: u64) -> JsResult<Value> {
    evaluate_program_with_limits(
        program,
        ExecutionLimits {
            step_limit,
            ..ExecutionLimits::default()
        },
    )
}

/// Evaluates a program with caller-provided resource limits, in a fresh
/// global state that is discarded afterwards.
pub fn evaluate_program_with_limits(program: &Program, limits: ExecutionLimits) -> JsResult<Value> {
    GlobalState::new().evaluate(program, limits)
}

/// The persistent global environment of one realm.
///
/// Globals live in two layers so scripts can be evaluated repeatedly:
/// `object` holds host globals plus `var` and function declarations, and
/// `lexical` (a child of `object`) holds top-level `let` and `const`. Lookups
/// check `lexical` first, so a `let` shadows a host global of the same name.
pub(crate) struct GlobalState {
    object: ScopeRef,
    lexical: ScopeRef,
}

impl GlobalState {
    /// Creates an empty global state with no host or script bindings.
    pub(crate) fn new() -> Self {
        let object: ScopeRef = Arc::default();
        let lexical = child_scope(&object);
        Self { object, lexical }
    }

    /// Binds a host-provided global, replacing any binding in the object
    /// layer. A script `let`/`const` of the same name still shadows it.
    pub(crate) fn install_host(&self, name: &str, value: Value) {
        self.object.lock_scope().bindings.insert(
            name.to_owned(),
            Binding {
                value: Some(value),
                mutable: true,
                lexical: false,
                host: true,
            },
        );
    }

    /// Evaluates `program` against the persistent globals.
    ///
    /// Declarations are validated before any is applied, so a redeclaration
    /// error leaves the globals untouched. Errors thrown while running keep
    /// the effects that happened before them, as in JavaScript.
    pub(crate) fn evaluate(&self, program: &Program, limits: ExecutionLimits) -> JsResult<Value> {
        with_engine_stack(|| {
            self.declare_globals(&program.body)?;
            let completion =
                Interpreter::new(limits).execute_statements(&self.lexical, &program.body)?;
            Ok(match completion {
                Completion::Normal(value) => value.unwrap_or(Value::Undefined),
                Completion::Return(value) => value,
            })
        })
    }

    /// Global declaration instantiation: checks every conflict first, then
    /// hoists `var`s, functions, and uninitialized `let`/`const` bindings.
    fn declare_globals(&self, statements: &[Statement]) -> JsResult<()> {
        let probe: ScopeRef = Arc::default();
        hoist_var_declarations(&probe, statements)?;
        let mut var_names: Vec<String> = probe.lock_scope().bindings.keys().cloned().collect();
        let mut lexical_names: Vec<&str> = Vec::new();
        for statement in statements {
            match statement {
                Statement::FunctionDeclaration { name, .. } => var_names.push(name.clone()),
                Statement::VariableDeclaration {
                    kind: VarKind::Let | VarKind::Const,
                    name,
                    ..
                } => lexical_names.push(name),
                _ => {}
            }
        }

        {
            let lexical = self.lexical.lock_scope();
            let object = self.object.lock_scope();
            for (index, name) in lexical_names.iter().enumerate() {
                let conflicts = lexical_names[..index].contains(name)
                    || var_names.iter().any(|var| var == name)
                    || lexical.bindings.contains_key(*name)
                    || object
                        .bindings
                        .get(*name)
                        .is_some_and(|binding| !binding.host);
                if conflicts {
                    return Err(already_declared(name));
                }
            }
            if let Some(name) = var_names
                .iter()
                .find(|name| lexical.bindings.contains_key(name.as_str()))
            {
                return Err(already_declared(name));
            }
        }

        for name in &var_names {
            declare_var(&self.object, name)?;
        }
        for statement in statements {
            match statement {
                Statement::VariableDeclaration {
                    kind: kind @ (VarKind::Let | VarKind::Const),
                    name,
                    ..
                } => declare_lexical(&self.lexical, name, *kind == VarKind::Let, None)?,
                Statement::FunctionDeclaration { name, params, body } => {
                    let function = Value::Function(Function(Arc::new(FunctionKind::Script {
                        name: name.clone(),
                        params: params.clone(),
                        body: body.clone(),
                        closure: Arc::clone(&self.lexical),
                    })));
                    initialize(&self.object, name, function);
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// Evaluates one expression in a fresh, empty environment.
///
/// Runs on the engine thread described in the module documentation and
/// stops after [`DEFAULT_STEP_LIMIT`] steps.
pub fn evaluate(expression: &Expr) -> JsResult<Value> {
    with_engine_stack(|| {
        Interpreter::new(ExecutionLimits::default()).evaluate(expression, &Arc::default())
    })
}

/// Statement result; `Normal(None)` is an empty completion, such as from a
/// declaration, which does not replace the previous completion value.
enum Completion {
    Normal(Option<Value>),
    Return(Value),
}

struct Interpreter {
    stack: StackGuard,
    steps_remaining: u64,
    call_depth: u64,
    loop_iterations_remaining: u64,
    call_depth_limit: u64,
}

impl Interpreter {
    /// Creates an interpreter measuring stack use from the caller's frame and
    /// allowing `step_limit` evaluation steps.
    fn new(limits: ExecutionLimits) -> Self {
        Self {
            stack: StackGuard::new(),
            steps_remaining: limits.step_limit,
            call_depth: 0,
            loop_iterations_remaining: limits.loop_iteration_limit,
            call_depth_limit: limits.call_depth_limit,
        }
    }

    /// Charges one step and checks stack use before evaluating a statement or
    /// expression.
    fn guarded<T>(&mut self, action: impl FnOnce(&mut Self) -> JsResult<T>) -> JsResult<T> {
        if self.steps_remaining == 0 {
            return Err(JsError::with_context(
                crate::JsErrorCategory::Limit,
                STEP_LIMIT_EXCEEDED,
                "evaluation step budget",
            ));
        }
        self.steps_remaining -= 1;
        if self.stack.exhausted() {
            return Err(JsError::with_context(
                crate::JsErrorCategory::Limit,
                STACK_OVERFLOW,
                "call stack",
            ));
        }
        action(self)
    }

    fn next_loop_iteration(&mut self) -> JsResult<()> {
        if self.loop_iterations_remaining == 0 {
            return Err(JsError::with_context(
                crate::JsErrorCategory::Limit,
                "Script exceeded the loop iteration limit",
                "while loop iteration",
            ));
        }
        self.loop_iterations_remaining -= 1;
        Ok(())
    }

    /// Runs a script or function body after hoisting its declarations.
    fn run_body(&mut self, scope: &ScopeRef, statements: &[Statement]) -> JsResult<Completion> {
        hoist_var_declarations(scope, statements)?;
        hoist_lexical_declarations(scope, statements, true)?;
        self.execute_statements(scope, statements)
    }

    fn execute_statements(
        &mut self,
        scope: &ScopeRef,
        statements: &[Statement],
    ) -> JsResult<Completion> {
        let mut value = None;
        for statement in statements {
            match self.execute_statement(scope, statement)? {
                Completion::Normal(Some(result)) => value = Some(result),
                Completion::Normal(None) => {}
                completion @ Completion::Return(_) => return Ok(completion),
            }
        }
        Ok(Completion::Normal(value))
    }

    fn execute_statement(
        &mut self,
        scope: &ScopeRef,
        statement: &Statement,
    ) -> JsResult<Completion> {
        self.guarded(|interpreter| interpreter.execute_statement_unguarded(scope, statement))
    }

    fn execute_statement_unguarded(
        &mut self,
        scope: &ScopeRef,
        statement: &Statement,
    ) -> JsResult<Completion> {
        match statement {
            Statement::Expression(expression) => {
                Ok(Completion::Normal(Some(self.evaluate(expression, scope)?)))
            }
            Statement::VariableDeclaration { kind, name, init } => {
                let value = match init {
                    Some(expression) => Some(self.evaluate(expression, scope)?),
                    None => None,
                };
                match (kind, value) {
                    (VarKind::Var, Some(value)) => assign(scope, name, value)?,
                    (VarKind::Var, None) => {}
                    (VarKind::Let | VarKind::Const, value) => {
                        initialize(scope, name, value.unwrap_or(Value::Undefined));
                    }
                }
                Ok(Completion::Normal(None))
            }
            Statement::Block(statements) => {
                let block_scope = child_scope(scope);
                hoist_lexical_declarations(&block_scope, statements, false)?;
                self.execute_statements(&block_scope, statements)
            }
            Statement::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let branch = if is_truthy(&self.evaluate(condition, scope)?) {
                    Some(then_branch)
                } else {
                    else_branch.as_ref()
                };
                let completion = match branch {
                    Some(branch) => self.execute_statement(scope, branch)?,
                    None => Completion::Normal(None),
                };
                Ok(match completion {
                    Completion::Normal(value) => {
                        Completion::Normal(Some(value.unwrap_or(Value::Undefined)))
                    }
                    completion => completion,
                })
            }
            Statement::While { condition, body } => {
                let mut last = Value::Undefined;
                while is_truthy(&self.evaluate(condition, scope)?) {
                    self.next_loop_iteration()?;
                    match self.execute_statement(scope, body)? {
                        Completion::Normal(Some(value)) => last = value,
                        Completion::Normal(None) => {}
                        completion @ Completion::Return(_) => return Ok(completion),
                    }
                }
                Ok(Completion::Normal(Some(last)))
            }
            // Bound when the enclosing body or block was entered.
            Statement::FunctionDeclaration { .. } => Ok(Completion::Normal(None)),
            Statement::ClassDeclaration(_) => Err(unsupported_here("class declarations")),
            Statement::Return(expression) => Ok(Completion::Return(match expression {
                Some(expression) => self.evaluate(expression, scope)?,
                None => Value::Undefined,
            })),
        }
    }

    fn evaluate(&mut self, expression: &Expr, scope: &ScopeRef) -> JsResult<Value> {
        self.guarded(|interpreter| interpreter.evaluate_unguarded(expression, scope))
    }

    fn evaluate_unguarded(&mut self, expression: &Expr, scope: &ScopeRef) -> JsResult<Value> {
        match expression {
            // Methods never receive `this` in this evaluator, so it reads like
            // any other undeclared name.
            Expr::This => Err(JsError::new("`this` is not defined")),
            Expr::New { .. } => Err(unsupported_here("`new`")),
            Expr::SuperCall { .. } | Expr::SuperMember { .. } => Err(unsupported_here("`super`")),
            Expr::Identifier(name) => read(scope, name),
            Expr::Assign { name, value } => {
                let value = self.evaluate(value, scope)?;
                assign(scope, name, value.clone())?;
                Ok(value)
            }
            Expr::Number(number) => Ok(Value::Number(*number)),
            Expr::String(text) => Ok(Value::String(text.clone())),
            Expr::Boolean(value) => Ok(Value::Boolean(*value)),
            Expr::Null => Ok(Value::Null),
            Expr::Undefined => Ok(Value::Undefined),
            Expr::Unary { operator, operand } => {
                let value = self.evaluate(operand, scope)?;
                Ok(match operator {
                    UnaryOperator::Negate => Value::Number(-to_number(&value)),
                    UnaryOperator::Not => Value::Boolean(!is_truthy(&value)),
                    UnaryOperator::BitwiseNot => Value::Number(f64::from(!to_int32(&value))),
                })
            }
            Expr::Binary {
                left,
                operator,
                right,
            } => {
                let left = self.evaluate(left, scope)?;
                let right = self.evaluate(right, scope)?;
                Ok(binary(*operator, &left, &right))
            }
            Expr::Logical {
                left,
                operator,
                right,
            } => {
                let left = self.evaluate(left, scope)?;
                let use_right = match operator {
                    LogicalOperator::And => is_truthy(&left),
                    LogicalOperator::Or => !is_truthy(&left),
                };
                if use_right {
                    self.evaluate(right, scope)
                } else {
                    Ok(left)
                }
            }
            Expr::Call { callee, arguments } => {
                let function = self.evaluate(callee, scope)?;
                let arguments = arguments
                    .iter()
                    .map(|argument| self.evaluate(argument, scope))
                    .collect::<JsResult<Vec<_>>>()?;
                let Value::Function(function) = function else {
                    let description = match describe(callee) {
                        Some(path) => format!("`{path}`"),
                        None => "expression".to_owned(),
                    };
                    return Err(JsError::with_context(
                        crate::JsErrorCategory::Runtime,
                        format!("{description} is not a function"),
                        "function call",
                    ));
                };
                self.call(&function, &arguments)
            }
            Expr::Member { object, property } => {
                let target = self.evaluate(object, scope)?;
                let key = self.property_key(property, scope)?;
                let Value::Object(target) = target else {
                    return Err(non_object(&target, "read", &key, "property access"));
                };
                target.get(&key).ok_or_else(|| {
                    let description = match describe(expression) {
                        Some(path) => format!("`{path}`"),
                        None => format!("Property `{key}`"),
                    };
                    JsError::with_context(
                        crate::JsErrorCategory::Runtime,
                        format!("{description} is not defined"),
                        "property access",
                    )
                })
            }
            Expr::MemberAssign {
                object,
                property,
                value,
            } => {
                let target = self.evaluate(object, scope)?;
                let key = self.property_key(property, scope)?;
                let value = self.evaluate(value, scope)?;
                let Value::Object(target) = target else {
                    return Err(non_object(&target, "set", &key, "property assignment"));
                };
                target.set(key, value.clone());
                Ok(value)
            }
        }
    }

    /// Evaluates a member expression's property to its string key.
    fn property_key(&mut self, property: &MemberProperty, scope: &ScopeRef) -> JsResult<String> {
        match property {
            MemberProperty::Named(name) => Ok(name.clone()),
            MemberProperty::Computed(key) => Ok(to_property_key(&self.evaluate(key, scope)?)),
        }
    }

    fn call(&mut self, function: &Function, arguments: &[Value]) -> JsResult<Value> {
        match &*function.0 {
            FunctionKind::Host { function, .. } => function(arguments),
            FunctionKind::Bytecode { .. } => Err(JsError::runtime(
                "bytecode functions can only be called by the bytecode VM",
            )),
            FunctionKind::Script {
                params,
                body,
                closure,
                ..
            } => self.guarded(|interpreter| {
                if interpreter.call_depth >= interpreter.call_depth_limit {
                    return Err(JsError::with_context(
                        crate::JsErrorCategory::Limit,
                        STACK_OVERFLOW,
                        "script function call depth",
                    ));
                }
                interpreter.call_depth += 1;
                let scope = child_scope(closure);
                for (index, param) in params.iter().enumerate() {
                    declare_var(&scope, param)?;
                    initialize(
                        &scope,
                        param,
                        arguments.get(index).cloned().unwrap_or(Value::Undefined),
                    );
                }
                let result = match interpreter.run_body(&scope, body)? {
                    Completion::Return(value) => value,
                    Completion::Normal(_) => Value::Undefined,
                };
                interpreter.call_depth -= 1;
                Ok(result)
            }),
        }
    }
}

/// The error for class syntax, which only the bytecode VM can run.
fn unsupported_here(construct: &str) -> JsError {
    JsError::with_context(
        crate::JsErrorCategory::Runtime,
        format!("{construct} is only supported by the bytecode compiler"),
        "tree-walk evaluator",
    )
}

/// Describes an identifier or a chain of `.name` accesses on one, such as
/// `console.log`, for error messages. Other expressions return `None`.
fn describe(expression: &Expr) -> Option<String> {
    match expression {
        Expr::Identifier(name) => Some(name.clone()),
        Expr::Member {
            object,
            property: MemberProperty::Named(name),
        } => describe(object).map(|path| format!("{path}.{name}")),
        _ => None,
    }
}

/// The error for reading or setting property `key` of a non-object value.
pub(crate) fn non_object(value: &Value, action: &str, key: &str, context: &str) -> JsError {
    let kind = match value {
        Value::Undefined => "undefined",
        Value::Null => "null",
        Value::Boolean(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Function(_) => "a function",
        Value::Object(_) => "an object",
    };
    JsError::with_context(
        crate::JsErrorCategory::Runtime,
        format!("Cannot {action} property `{key}` of {kind}"),
        context,
    )
}

/// Declares every `var` in a function or script body, including those nested
/// in blocks and control flow, but not those inside nested functions.
fn hoist_var_declarations(scope: &ScopeRef, statements: &[Statement]) -> JsResult<()> {
    for statement in statements {
        match statement {
            Statement::VariableDeclaration {
                kind: VarKind::Var,
                name,
                ..
            } => declare_var(scope, name)?,
            Statement::Block(statements) => hoist_var_declarations(scope, statements)?,
            Statement::If {
                then_branch,
                else_branch,
                ..
            } => {
                hoist_var_declarations(scope, std::slice::from_ref(then_branch))?;
                if let Some(else_branch) = else_branch {
                    hoist_var_declarations(scope, std::slice::from_ref(else_branch))?;
                }
            }
            Statement::While { body, .. } => {
                hoist_var_declarations(scope, std::slice::from_ref(body))?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Declares the `let`, `const`, and function declarations that appear
/// directly in `statements`. `let`/`const` start uninitialized. Functions are
/// bound immediately: `var`-style in a function or script body, block scoped
/// otherwise.
fn hoist_lexical_declarations(
    scope: &ScopeRef,
    statements: &[Statement],
    function_body: bool,
) -> JsResult<()> {
    for statement in statements {
        match statement {
            Statement::VariableDeclaration {
                kind: kind @ (VarKind::Let | VarKind::Const),
                name,
                ..
            } => declare_lexical(scope, name, *kind == VarKind::Let, None)?,
            Statement::FunctionDeclaration { name, params, body } => {
                let function = Value::Function(Function(Arc::new(FunctionKind::Script {
                    name: name.clone(),
                    params: params.clone(),
                    body: body.clone(),
                    closure: Arc::clone(scope),
                })));
                if function_body {
                    declare_var(scope, name)?;
                    initialize(scope, name, function);
                } else {
                    declare_lexical(scope, name, true, Some(function))?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn binary(operator: BinaryOperator, left: &Value, right: &Value) -> Value {
    let number = |operation: fn(f64, f64) -> f64| {
        Value::Number(operation(to_number(left), to_number(right)))
    };
    let int32 = |operation: fn(i32, i32) -> i32| {
        Value::Number(f64::from(operation(to_int32(left), to_int32(right))))
    };
    let shift = to_uint32(right) & 31;
    match operator {
        BinaryOperator::Add => {
            let (left, right) = (to_primitive(left), to_primitive(right));
            if matches!(left, Value::String(_)) || matches!(right, Value::String(_)) {
                Value::String(format!("{left}{right}"))
            } else {
                Value::Number(to_number(&left) + to_number(&right))
            }
        }
        BinaryOperator::Subtract => number(|left, right| left - right),
        BinaryOperator::Multiply => number(|left, right| left * right),
        BinaryOperator::Divide => number(|left, right| left / right),
        BinaryOperator::Remainder => number(|left, right| left % right),
        BinaryOperator::BitwiseAnd => int32(|left, right| left & right),
        BinaryOperator::BitwiseOr => int32(|left, right| left | right),
        BinaryOperator::BitwiseXor => int32(|left, right| left ^ right),
        BinaryOperator::LeftShift => Value::Number(f64::from(to_int32(left).wrapping_shl(shift))),
        BinaryOperator::SignedRightShift => Value::Number(f64::from(to_int32(left) >> shift)),
        BinaryOperator::UnsignedRightShift => Value::Number(f64::from(to_uint32(left) >> shift)),
        BinaryOperator::Less => Value::Boolean(less_than(left, right) == Some(true)),
        BinaryOperator::Greater => Value::Boolean(less_than(right, left) == Some(true)),
        BinaryOperator::LessEqual => Value::Boolean(less_than(right, left) == Some(false)),
        BinaryOperator::GreaterEqual => Value::Boolean(less_than(left, right) == Some(false)),
        BinaryOperator::Equal => Value::Boolean(loose_equals(left, right)),
        BinaryOperator::NotEqual => Value::Boolean(!loose_equals(left, right)),
        BinaryOperator::StrictEqual => Value::Boolean(strict_equals(left, right)),
        BinaryOperator::StrictNotEqual => Value::Boolean(!strict_equals(left, right)),
    }
}
