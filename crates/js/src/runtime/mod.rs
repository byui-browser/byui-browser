//! Tree-walk interpreter for parsed programs.
//!
//! Supports expressions, `var`/`let`/`const` declarations, blocks, `if`,
//! `while`, function declarations with closures, calls, `return`, object
//! literals, and member access (`object.name`, `object[key]`, and assignment
//! to either).
//!
//! Semantics follow ECMAScript where supported: `var` and function
//! declarations are hoisted to the enclosing function or script, `let` and
//! `const` are block scoped and unusable before their declaration runs, and
//! operators use JavaScript type conversions.
//!
//! The conditional, comma, update, compound-assignment, `??`, `typeof`,
//! `void`, `delete`, `in`, `instanceof`, `**`, template, spread, and
//! optional-chaining expression forms are implemented here with the same
//! semantics as the bytecode VM; see `docs/js/expression-forms.md`.
//!
//! Limitations:
//! - Assigning to an undeclared name is an error, as in strict mode.
//! - Member access works only on [`Object`] values. Reading a property of any
//!   other value, including strings and functions, is a runtime error.
//!   Reading a property an object does not have returns `undefined`, as in
//!   JavaScript. Methods are called without `this`.
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

mod array;
pub(crate) mod builtins;
mod conversions;
mod expressions;
mod object;
mod operators;

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::ast::{
    ArrowBody, AssignmentOperator, BinaryOperator, Expr, LogicalOperator, MemberProperty, Program,
    PropertyKey, Statement, UnaryOperator, UpdateOperator, VarKind,
};
use crate::bytecode::FunctionTemplate;
use crate::stack::{StackGuard, with_engine_stack};
use crate::{HostFunction, JsError, JsResult, Value};
pub use array::MAX_ARRAY_LENGTH;
pub(crate) use array::{array_len, array_to_string, get_array_property, set_array_property};
pub(crate) use conversions::{is_truthy, number_to_string, to_property_key};
pub(crate) use expressions::{copy_data_properties, delete_property, is_nullish, spread_values};
pub use object::{
    HostGetter, HostPropertyGetter, HostPropertyKeys, HostPropertySetter, HostSetter, Object,
    WeakObject,
};
pub(crate) use operators::{binary, unary};

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
    setter: Option<HostSetter>,
}

/// A shared bytecode binding cell retained by closures.
#[derive(Clone, Debug)]
pub(crate) struct BytecodeCapture {
    pub(crate) value: Arc<Mutex<Option<Value>>>,
    pub(crate) kind: Option<VarKind>,
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

/// Whether `name` is bound in `scope` or an enclosing one, even if it is
/// still uninitialized.
fn is_declared(scope: &ScopeRef, name: &str) -> bool {
    with_binding(scope, name, |_| Ok(())).is_ok()
}

/// Reads `target[key]` for an object or array; any other value is an error.
fn read_member(target: &Value, key: &str) -> JsResult<Value> {
    match target {
        Value::Object(target) => Ok(target.get_property(key)?.unwrap_or(Value::Undefined)),
        Value::Array(target) => Ok(get_array_property(target, key)),
        target => Err(non_object(target, "read", key, "property access")),
    }
}

/// Writes `target[key] = value` for an object or array; any other value is
/// an error.
fn write_member(target: &Value, key: String, value: Value) -> JsResult<()> {
    match target {
        Value::Object(target) => target.set_property(key, value),
        Value::Array(target) => set_array_property(target, &key, value),
        target => Err(non_object(target, "set", &key, "property assignment")),
    }
}

fn assign(scope: &ScopeRef, name: &str, value: Value) -> JsResult<()> {
    with_binding(scope, name, |binding| {
        if binding.value.is_none() {
            return Err(uninitialized(name));
        }
        if !binding.mutable {
            return Err(JsError::new(format!("Assignment to constant `{name}`")));
        }
        if let Some(setter) = &binding.setter {
            return setter(value);
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
                    setter: None,
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
            setter: None,
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
        constructor: Option<HostFunction>,
    },
    /// A function compiled to bytecode; only the VM can run it.
    Bytecode {
        template: Arc<FunctionTemplate>,
        captures: Arc<Vec<BytecodeCapture>>,
        lexical_this: Option<Value>,
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
            constructor: None,
        }))
    }

    /// Wraps a Rust callback as a callable and constructible JavaScript
    /// function. The constructor callback receives the arguments passed to
    /// `new` and must return the constructed value.
    pub(crate) fn from_host_constructor(
        name: impl Into<String>,
        function: HostFunction,
        constructor: HostFunction,
    ) -> Self {
        Self(Arc::new(FunctionKind::Host {
            name: name.into(),
            function,
            constructor: Some(constructor),
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
        Self::from_template_with_captures(
            template,
            home_object,
            parent,
            prototype,
            Vec::new(),
            None,
        )
    }

    /// Creates a bytecode function with captured lexical bindings.
    pub(crate) fn from_template_with_captures(
        template: Arc<FunctionTemplate>,
        home_object: Option<Object>,
        parent: Option<Function>,
        prototype: Option<Object>,
        captures: Vec<BytecodeCapture>,
        lexical_this: Option<Value>,
    ) -> Self {
        let constructible = template.kind.is_constructor();
        let function = Self(Arc::new(FunctionKind::Bytecode {
            template,
            captures: Arc::new(captures),
            lexical_this,
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

    pub(crate) fn captures(&self) -> Option<&Arc<Vec<BytecodeCapture>>> {
        match &*self.0 {
            FunctionKind::Bytecode { captures, .. } => Some(captures),
            _ => None,
        }
    }

    pub(crate) fn lexical_this(&self) -> Option<Value> {
        match &*self.0 {
            FunctionKind::Bytecode { lexical_this, .. } => lexical_this.clone(),
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

    /// Returns an own property of a function, such as a class's `prototype`.
    ///
    /// Functions created by the bytecode VM expose own properties. Functions
    /// created by the tree-walk evaluator or host API currently do not.
    pub fn get_own_property(&self, key: &str) -> Option<Value> {
        self.get_own(key)
    }

    /// Sets an own property on a bytecode function.
    ///
    /// Returns `false` for host and tree-walk functions, whose properties are
    /// not currently modeled.
    pub fn set_own_property(&self, key: &str, value: Value) -> bool {
        self.set_own(key, value)
    }

    /// Returns whether this function was compiled for the bytecode VM.
    pub fn is_bytecode_function(&self) -> bool {
        self.template().is_some()
    }

    /// Constructs an instance by applying `new` to this function.
    ///
    /// Host functions use their constructor callback and fail if they have
    /// none. Bytecode functions run in a fresh VM; that standalone
    /// construction does not retain the VM's global environment, so a
    /// constructor that reads global variables may fail. Tree-walk functions
    /// run in a fresh interpreter with a new plain object as `this`; they have
    /// no `prototype` property, so the instance has no prototype.
    pub fn construct(&self, arguments: &[Value]) -> JsResult<Value> {
        match &*self.0 {
            FunctionKind::Host {
                constructor: Some(constructor),
                ..
            } => constructor(arguments),
            FunctionKind::Host { .. } => Err(JsError::runtime("function is not a constructor")),
            FunctionKind::Script { .. } => with_engine_stack(|| {
                Interpreter::new(ExecutionLimits::default()).construct(self, arguments)
            }),
            // The VM only hands bytecode functions back to this method's
            // callers, so this cannot recurse into another `construct`.
            FunctionKind::Bytecode { .. } => crate::Vm::construct_function(self.clone(), arguments),
        }
    }

    /// Calls this function with an explicit receiver using a fresh bytecode
    /// VM. Host and tree-walk functions use their existing call mechanism.
    pub fn call_with_this(&self, receiver: Value, arguments: &[Value]) -> JsResult<Value> {
        if self.template().is_some() {
            crate::Vm::call_function(self.clone(), receiver, arguments)
        } else {
            with_engine_stack(|| {
                let mut interpreter = Interpreter::new(ExecutionLimits::default());
                interpreter.this_enabled = true;
                interpreter.call_with_this(self, receiver, arguments)
            })
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

    /// Invokes this function with `arguments`.
    ///
    /// Script functions execute with their captured lexical environment, and
    /// host functions receive the supplied values directly. This is used by
    /// host APIs that need to call a JavaScript callback, such as DOM events.
    /// Bytecode functions are not run here; the VM executes those.
    pub fn call(&self, arguments: &[Value]) -> JsResult<Value> {
        with_engine_stack(|| Interpreter::new(ExecutionLimits::default()).call(self, arguments))
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
#[derive(Clone)]
pub(crate) struct GlobalState {
    object: ScopeRef,
    lexical: ScopeRef,
}

impl GlobalState {
    /// Creates an empty global state with no host or script bindings.
    pub(crate) fn new() -> Self {
        let object: ScopeRef = Arc::default();
        let lexical = child_scope(&object);
        let state = Self { object, lexical };
        builtins::install(&state);
        state
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
                setter: None,
            },
        );
    }

    /// Sets the host hook that handles assignment to a registered global.
    pub(crate) fn set_host_setter(&self, name: &str, setter: HostSetter) -> JsResult<()> {
        let mut object = self.object.lock_scope();
        let binding = object
            .bindings
            .get_mut(name)
            .ok_or_else(|| JsError::new(format!("host global `{name}` is not registered")))?;
        if !binding.host {
            return Err(JsError::new(format!("`{name}` is not a host global")));
        }
        binding.setter = Some(setter);
        Ok(())
    }

    /// Evaluates `program` against the persistent globals.
    ///
    /// Declarations are validated before any is applied, so a redeclaration
    /// error leaves the globals untouched. Errors thrown while running keep
    /// the effects that happened before them, as in JavaScript.
    pub(crate) fn evaluate(&self, program: &Program, limits: ExecutionLimits) -> JsResult<Value> {
        self.evaluate_with_this_enabled(program, limits, Value::Undefined, false)
    }

    /// Evaluates `program` with a host-specified global `this` value.
    pub(crate) fn evaluate_with_this(
        &self,
        program: &Program,
        limits: ExecutionLimits,
        this_value: Value,
    ) -> JsResult<Value> {
        self.evaluate_with_this_enabled(program, limits, this_value, true)
    }

    fn evaluate_with_this_enabled(
        &self,
        program: &Program,
        limits: ExecutionLimits,
        this_value: Value,
        this_enabled: bool,
    ) -> JsResult<Value> {
        log::trace!("evaluating {} top-level statements", program.body.len());
        with_engine_stack(|| {
            self.declare_globals(&program.body)?;
            let mut interpreter = Interpreter::new(limits);
            interpreter.this_value = this_value;
            interpreter.this_enabled = this_enabled;
            let completion = interpreter.execute_statements(&self.lexical, &program.body)?;
            Ok(match completion {
                Completion::Normal(value) => value.unwrap_or(Value::Undefined),
                Completion::Return(value) => value,
                Completion::Break | Completion::Continue => Value::Undefined,
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
    /// `break` on its way to the innermost loop.
    Break,
    /// `continue` on its way to the innermost loop.
    Continue,
}

struct Interpreter {
    stack: StackGuard,
    steps_remaining: u64,
    call_depth: u64,
    loop_iterations_remaining: u64,
    call_depth_limit: u64,
    this_value: Value,
    this_enabled: bool,
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
            this_value: Value::Undefined,
            this_enabled: false,
        }
    }

    /// Charges one step and checks stack use before evaluating a statement or
    /// expression.
    fn guarded<T>(&mut self, action: impl FnOnce(&mut Self) -> JsResult<T>) -> JsResult<T> {
        if self.steps_remaining == 0 {
            log::warn!("evaluation step limit reached in the tree-walk interpreter");
            return Err(JsError::with_context(
                crate::JsErrorCategory::Limit,
                STEP_LIMIT_EXCEEDED,
                "evaluation step budget",
            ));
        }
        self.steps_remaining -= 1;
        if self.stack.exhausted() {
            log::warn!("native stack budget exhausted in the tree-walk interpreter");
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
            log::warn!("loop iteration limit reached in the tree-walk interpreter");
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
                completion => return Ok(completion),
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
                        Completion::Normal(None) | Completion::Continue => {}
                        Completion::Break => break,
                        completion @ Completion::Return(_) => return Ok(completion),
                    }
                }
                Ok(Completion::Normal(Some(last)))
            }
            Statement::For {
                init,
                condition,
                update,
                body,
            } => self.execute_for_statement(scope, init, condition, update, body),
            // Bound when the enclosing body or block was entered.
            Statement::FunctionDeclaration { .. } => Ok(Completion::Normal(None)),
            Statement::ClassDeclaration(_) => Err(unsupported_here("class declarations")),
            Statement::Return(expression) => Ok(Completion::Return(match expression {
                Some(expression) => self.evaluate(expression, scope)?,
                None => Value::Undefined,
            })),
            Statement::Throw(expression) => {
                let value = self.evaluate(expression, scope)?;
                Err(JsError::rethrown(value))
            }
            Statement::Break => Ok(Completion::Break),
            Statement::Continue => Ok(Completion::Continue),
            Statement::Try {
                block,
                handler,
                finalizer,
            } => self.execute_try(scope, block, handler.as_ref(), finalizer.as_deref()),
        }
    }

    fn execute_for_statement(
        &mut self,
        scope: &ScopeRef,
        init: &Option<Box<Statement>>,
        condition: &Option<Expr>,
        update: &Option<Expr>,
        body: &Statement,
    ) -> JsResult<Completion> {
        let loop_scope = match init.as_deref() {
            Some(Statement::VariableDeclaration {
                kind: VarKind::Let | VarKind::Const,
                ..
            }) => child_scope(scope),
            _ => Arc::clone(scope),
        };
        if let Some(init) = init {
            if !Arc::ptr_eq(&loop_scope, scope) {
                hoist_lexical_declarations(&loop_scope, std::slice::from_ref(init), false)?;
            }
            self.execute_statement(&loop_scope, init)?;
        }
        let mut last = Value::Undefined;
        while condition
            .as_ref()
            .map(|condition| {
                self.evaluate(condition, &loop_scope)
                    .map(|value| is_truthy(&value))
            })
            .transpose()?
            .unwrap_or(true)
        {
            self.next_loop_iteration()?;
            match self.execute_statement(&loop_scope, body)? {
                Completion::Normal(Some(value)) => last = value,
                Completion::Normal(None) | Completion::Continue => {}
                Completion::Break => break,
                completion @ Completion::Return(_) => return Ok(completion),
            }
            if let Some(update) = update {
                self.evaluate(update, &loop_scope)?;
            }
        }
        Ok(Completion::Normal(Some(last)))
    }

    /// Runs a block in its own lexical scope.
    fn execute_block(
        &mut self,
        scope: &ScopeRef,
        statements: &[Statement],
    ) -> JsResult<Completion> {
        let block_scope = child_scope(scope);
        hoist_lexical_declarations(&block_scope, statements, false)?;
        self.execute_statements(&block_scope, statements)
    }

    /// Runs `try`, `catch`, and `finally` with the same semantics as the
    /// bytecode VM: a catchable error from the `try` block goes to `catch`;
    /// `finally` then runs on every exit, and completing abruptly (an error,
    /// `return`, `break`, or `continue`) replaces the pending outcome. Limit
    /// errors skip both clauses because they end the script.
    fn execute_try(
        &mut self,
        scope: &ScopeRef,
        block: &[Statement],
        handler: Option<&crate::ast::CatchClause>,
        finalizer: Option<&[Statement]>,
    ) -> JsResult<Completion> {
        let mut outcome = self.execute_block(scope, block);
        if let (Some(clause), Err(error)) = (handler, &outcome)
            && error.category.is_catchable()
        {
            let caught = error.clone().into_caught_value();
            let catch_scope = child_scope(scope);
            if let Some(param) = &clause.param {
                catch_scope.lock_scope().bindings.insert(
                    param.clone(),
                    Binding {
                        value: Some(caught),
                        mutable: true,
                        lexical: true,
                        host: false,
                        setter: None,
                    },
                );
            }
            outcome = self.execute_block(&catch_scope, &clause.body);
        }
        if let Some(statements) = finalizer {
            if matches!(&outcome, Err(error) if !error.category.is_catchable()) {
                return outcome;
            }
            match self.execute_block(scope, statements)? {
                Completion::Normal(_) => {}
                abrupt => return Ok(abrupt),
            }
        }
        outcome.map(|completion| match completion {
            Completion::Normal(value) => {
                Completion::Normal(Some(value.unwrap_or(Value::Undefined)))
            }
            abrupt => abrupt,
        })
    }

    fn evaluate(&mut self, expression: &Expr, scope: &ScopeRef) -> JsResult<Value> {
        self.guarded(|interpreter| interpreter.evaluate_unguarded(expression, scope))
    }

    fn evaluate_unguarded(&mut self, expression: &Expr, scope: &ScopeRef) -> JsResult<Value> {
        match expression {
            // Methods never receive `this` in this evaluator, so it reads like
            // any other undeclared name.
            Expr::This if self.this_enabled => Ok(self.this_value.clone()),
            Expr::This => Err(JsError::new("`this` is not defined")),
            Expr::New { callee, arguments } => {
                let constructor = self.evaluate(callee, scope)?;
                let Value::Function(function) = constructor else {
                    return Err(JsError::with_context(
                        crate::JsErrorCategory::Runtime,
                        format!("`{constructor}` is not a constructor"),
                        "new expression",
                    ));
                };
                let arguments = arguments
                    .iter()
                    .map(|argument| self.evaluate(argument, scope))
                    .collect::<JsResult<Vec<_>>>()?;
                self.construct(&function, &arguments)
            }
            Expr::SuperCall { .. } | Expr::SuperMember { .. } => Err(unsupported_here("`super`")),
            Expr::Identifier(name) => read(scope, name),
            Expr::ArrowFunction { params, body } => {
                let body = match body {
                    ArrowBody::Expression(expression) => {
                        vec![Statement::Return(Some((**expression).clone()))]
                    }
                    ArrowBody::Block(statements) => statements.clone(),
                };
                Ok(Value::Function(Function(Arc::new(FunctionKind::Script {
                    name: String::new(),
                    params: params.clone(),
                    body,
                    closure: Arc::clone(scope),
                }))))
            }
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
                // `typeof missing` is `"undefined"`, not a reference error.
                if *operator == UnaryOperator::Typeof
                    && let Expr::Identifier(name) = &**operand
                    && !is_declared(scope, name)
                {
                    return Ok(Value::String("undefined".to_owned()));
                }
                let value = self.evaluate(operand, scope)?;
                Ok(unary(*operator, &value))
            }
            Expr::Binary {
                left,
                operator,
                right,
            } => {
                let left = self.evaluate(left, scope)?;
                let right = self.evaluate(right, scope)?;
                binary(*operator, &left, &right)
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
                    LogicalOperator::Nullish => is_nullish(&left),
                };
                if use_right {
                    self.evaluate(right, scope)
                } else {
                    Ok(left)
                }
            }
            Expr::Call { callee, arguments } => {
                let function = self.evaluate(callee, scope)?;
                let arguments = self.evaluate_arguments(arguments, scope)?;
                self.call_value(function, callee, &arguments)
            }
            Expr::Member { object, property } => {
                let target = self.evaluate(object, scope)?;
                let key = self.property_key(property, scope)?;
                read_member(&target, &key)
            }
            Expr::MemberAssign {
                object,
                property,
                value,
            } => {
                let target = self.evaluate(object, scope)?;
                let key = self.property_key(property, scope)?;
                let value = self.evaluate(value, scope)?;
                write_member(&target, key, value.clone())?;
                Ok(value)
            }
            Expr::Object(properties) => {
                let object = Object::new();
                for property in properties {
                    let key = match &property.key {
                        PropertyKey::Named(name) => name.clone(),
                        PropertyKey::Computed(key) => to_property_key(&self.evaluate(key, scope)?),
                        PropertyKey::Spread => {
                            let source = self.evaluate(&property.value, scope)?;
                            copy_data_properties(&object, &source)?;
                            continue;
                        }
                    };
                    let value = self.evaluate(&property.value, scope)?;
                    object.set(key, value);
                }
                Ok(Value::Object(object))
            }
            Expr::Array(elements) => {
                let array = Object::new();
                // A literal with a spread is built incrementally by the VM
                // from an empty array, whose `length` is already defined.
                // Define it first here too so both engines produce the same
                // property order.
                if elements
                    .iter()
                    .any(|element| matches!(element, Some(Expr::Spread(_))))
                {
                    array.set("length", Value::Number(0.0));
                }
                let mut length = 0usize;
                for element in elements {
                    match element {
                        Some(Expr::Spread(source)) => {
                            let source = self.evaluate(source, scope)?;
                            for value in spread_values(&source)? {
                                set_array_property(&array, &length.to_string(), value)?;
                                length += 1;
                            }
                        }
                        Some(element) => {
                            let value = self.evaluate(element, scope)?;
                            array.set(length.to_string(), value);
                            length += 1;
                        }
                        None => {
                            array.set(length.to_string(), Value::Undefined);
                            length += 1;
                        }
                    }
                }
                array.set("length", Value::Number(length as f64));
                Ok(Value::Array(array))
            }
            Expr::Conditional {
                test,
                consequent,
                alternate,
            } => {
                let test = self.evaluate(test, scope)?;
                if is_truthy(&test) {
                    self.evaluate(consequent, scope)
                } else {
                    self.evaluate(alternate, scope)
                }
            }
            Expr::Sequence(operands) => {
                let mut result = Value::Undefined;
                for operand in operands {
                    result = self.evaluate(operand, scope)?;
                }
                Ok(result)
            }
            Expr::Template {
                quasis,
                expressions,
            } => {
                let mut parts = quasis.iter();
                let mut text = parts.next().cloned().unwrap_or_default();
                for (expression, quasi) in expressions.iter().zip(parts) {
                    let value = self.evaluate(expression, scope)?;
                    text = binary(BinaryOperator::Add, &Value::String(text), &value)?.to_string();
                    text.push_str(quasi);
                }
                Ok(Value::String(text))
            }
            Expr::Update {
                target,
                operator,
                prefix,
            } => self.evaluate_update(target, *operator, *prefix, scope),
            Expr::CompoundAssign {
                target,
                operator,
                value,
            } => self.evaluate_compound(target, *operator, value, scope),
            Expr::Delete(operand) => match &**operand {
                Expr::Member { object, property } => {
                    let target = self.evaluate(object, scope)?;
                    let key = self.property_key(property, scope)?;
                    delete_property(&target, &key).map(Value::Boolean)
                }
                other => {
                    self.evaluate(other, scope)?;
                    Ok(Value::Boolean(true))
                }
            },
            Expr::OptionalChain(chain) => Ok(self
                .evaluate_chain(chain, scope)?
                .unwrap_or(Value::Undefined)),
            Expr::OptionalMember { .. } | Expr::OptionalCall { .. } => Err(JsError::runtime(
                "optional access outside an optional chain",
            )),
            Expr::Spread(_) => Err(JsError::runtime(
                "spread outside an array, call, or object literal",
            )),
        }
    }

    /// Evaluates the call arguments in order, expanding each spread.
    fn evaluate_arguments(&mut self, arguments: &[Expr], scope: &ScopeRef) -> JsResult<Vec<Value>> {
        let mut values = Vec::with_capacity(arguments.len());
        for argument in arguments {
            if let Expr::Spread(source) = argument {
                let source = self.evaluate(source, scope)?;
                values.extend(spread_values(&source)?);
            } else {
                values.push(self.evaluate(argument, scope)?);
            }
        }
        Ok(values)
    }

    /// Calls an already-evaluated callee, describing `callee` in the error
    /// when the value is not a function.
    fn call_value(
        &mut self,
        function: Value,
        callee: &Expr,
        arguments: &[Value],
    ) -> JsResult<Value> {
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
        self.call(&function, arguments)
    }

    /// Evaluates a link of an optional chain, returning `None` when a `?.`
    /// inside it found a `null` or `undefined` base.
    fn evaluate_chain(&mut self, expression: &Expr, scope: &ScopeRef) -> JsResult<Option<Value>> {
        match expression {
            Expr::OptionalMember { object, property } => {
                let Some(target) = self.evaluate_chain(object, scope)? else {
                    return Ok(None);
                };
                if is_nullish(&target) {
                    return Ok(None);
                }
                let key = self.property_key(property, scope)?;
                read_member(&target, &key).map(Some)
            }
            Expr::Member { object, property } => {
                let Some(target) = self.evaluate_chain(object, scope)? else {
                    return Ok(None);
                };
                let key = self.property_key(property, scope)?;
                read_member(&target, &key).map(Some)
            }
            Expr::OptionalCall { callee, arguments } => {
                let Some(function) = self.evaluate_chain(callee, scope)? else {
                    return Ok(None);
                };
                if is_nullish(&function) {
                    return Ok(None);
                }
                let arguments = self.evaluate_arguments(arguments, scope)?;
                self.call_value(function, callee, &arguments).map(Some)
            }
            Expr::Call { callee, arguments } => {
                let Some(function) = self.evaluate_chain(callee, scope)? else {
                    return Ok(None);
                };
                let arguments = self.evaluate_arguments(arguments, scope)?;
                self.call_value(function, callee, &arguments).map(Some)
            }
            other => self.evaluate(other, scope).map(Some),
        }
    }

    /// Evaluates `++x`, `x++`, `--x`, or `x--`.
    fn evaluate_update(
        &mut self,
        target: &Expr,
        operator: UpdateOperator,
        prefix: bool,
        scope: &ScopeRef,
    ) -> JsResult<Value> {
        let step = match operator {
            UpdateOperator::Increment => BinaryOperator::Add,
            UpdateOperator::Decrement => BinaryOperator::Subtract,
        };
        match target {
            Expr::Identifier(name) => {
                let old = unary(UnaryOperator::Plus, &read(scope, name)?);
                let new = binary(step, &old, &Value::Number(1.0))?;
                assign(scope, name, new.clone())?;
                Ok(if prefix { new } else { old })
            }
            Expr::Member { object, property } => {
                let target = self.evaluate(object, scope)?;
                let key = self.property_key(property, scope)?;
                let old = unary(UnaryOperator::Plus, &read_member(&target, &key)?);
                let new = binary(step, &old, &Value::Number(1.0))?;
                write_member(&target, key, new.clone())?;
                Ok(if prefix { new } else { old })
            }
            _ => Err(JsError::runtime("Invalid update target")),
        }
    }

    /// Evaluates a compound assignment such as `x += 1` or `o.a ??= 2`.
    fn evaluate_compound(
        &mut self,
        target: &Expr,
        operator: AssignmentOperator,
        value: &Expr,
        scope: &ScopeRef,
    ) -> JsResult<Value> {
        match target {
            Expr::Identifier(name) => {
                let old = read(scope, name)?;
                let Some(new) = self.combine(&old, operator, value, scope)? else {
                    return Ok(old);
                };
                assign(scope, name, new.clone())?;
                Ok(new)
            }
            Expr::Member { object, property } => {
                let target = self.evaluate(object, scope)?;
                let key = self.property_key(property, scope)?;
                let old = read_member(&target, &key)?;
                let Some(new) = self.combine(&old, operator, value, scope)? else {
                    return Ok(old);
                };
                write_member(&target, key, new.clone())?;
                Ok(new)
            }
            _ => Err(JsError::runtime("Invalid assignment target")),
        }
    }

    /// Computes the value a compound assignment stores, or `None` when a
    /// logical operator leaves the target alone without evaluating `value`.
    fn combine(
        &mut self,
        old: &Value,
        operator: AssignmentOperator,
        value: &Expr,
        scope: &ScopeRef,
    ) -> JsResult<Option<Value>> {
        match operator {
            AssignmentOperator::Binary(operator) => {
                let right = self.evaluate(value, scope)?;
                binary(operator, old, &right).map(Some)
            }
            AssignmentOperator::Logical(operator) => {
                let assigns = match operator {
                    LogicalOperator::And => is_truthy(old),
                    LogicalOperator::Or => !is_truthy(old),
                    LogicalOperator::Nullish => is_nullish(old),
                };
                if assigns {
                    self.evaluate(value, scope).map(Some)
                } else {
                    Ok(None)
                }
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
        self.call_with_this(function, Value::Undefined, arguments)
    }

    /// Applies `new` to `function`.
    ///
    /// Script functions run in this interpreter, so the call depth and stack
    /// limits keep covering them, with a fresh plain object as `this`. An
    /// object returned by the body replaces that instance, as in JavaScript.
    /// Other kinds are delegated to [`Function::construct`].
    fn construct(&mut self, function: &Function, arguments: &[Value]) -> JsResult<Value> {
        if !matches!(&*function.0, FunctionKind::Script { .. }) {
            return function.construct(arguments);
        }
        let instance = Value::Object(Object::new());
        let previous_this_enabled = std::mem::replace(&mut self.this_enabled, true);
        let result = self.call_with_this(function, instance.clone(), arguments);
        self.this_enabled = previous_this_enabled;
        Ok(match result? {
            returned @ (Value::Object(_) | Value::Array(_) | Value::Function(_)) => returned,
            _ => instance,
        })
    }

    fn call_with_this(
        &mut self,
        function: &Function,
        receiver: Value,
        arguments: &[Value],
    ) -> JsResult<Value> {
        match &*function.0 {
            FunctionKind::Host { function, .. } => {
                log::trace!("calling host function with {} arguments", arguments.len());
                function(arguments)
            }
            FunctionKind::Bytecode { .. } => {
                log::debug!("tree-walk interpreter asked to call a bytecode function");
                Err(JsError::runtime(
                    "bytecode functions can only be called by the bytecode VM",
                ))
            }
            FunctionKind::Script {
                params,
                body,
                closure,
                ..
            } => self.guarded(|interpreter| {
                if interpreter.call_depth >= interpreter.call_depth_limit {
                    log::warn!(
                        "call depth limit of {} reached in the tree-walk interpreter",
                        interpreter.call_depth_limit
                    );
                    return Err(JsError::with_context(
                        crate::JsErrorCategory::Limit,
                        STACK_OVERFLOW,
                        "script function call depth",
                    ));
                }
                interpreter.call_depth += 1;
                log::trace!(
                    "calling script function with {} arguments at depth {}",
                    arguments.len(),
                    interpreter.call_depth
                );
                let scope = child_scope(closure);
                let previous_this = std::mem::replace(&mut interpreter.this_value, receiver);
                for (index, param) in params.iter().enumerate() {
                    declare_var(&scope, param)?;
                    initialize(
                        &scope,
                        param,
                        arguments.get(index).cloned().unwrap_or(Value::Undefined),
                    );
                }
                // The depth must be restored on errors too, because a script
                // `catch` can resume execution in the caller.
                let result = interpreter.run_body(&scope, body);
                interpreter.this_value = previous_this;
                interpreter.call_depth -= 1;
                Ok(match result? {
                    Completion::Return(value) => value,
                    _ => Value::Undefined,
                })
            }),
        }
    }
}

/// The error for class syntax, which only the bytecode VM can run.
fn unsupported_here(construct: &str) -> JsError {
    log::warn!("tree-walk evaluator does not support {construct}");
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
        Value::Array(_) => "an array",
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
            Statement::For { init, body, .. } => {
                if let Some(init) = init {
                    hoist_var_declarations(scope, std::slice::from_ref(init))?;
                }
                hoist_var_declarations(scope, std::slice::from_ref(body))?;
            }
            Statement::Try {
                block,
                handler,
                finalizer,
            } => {
                hoist_var_declarations(scope, block)?;
                if let Some(clause) = handler {
                    hoist_var_declarations(scope, &clause.body)?;
                }
                if let Some(statements) = finalizer {
                    hoist_var_declarations(scope, statements)?;
                }
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
