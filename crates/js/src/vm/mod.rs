//! Stack-based virtual machine for executing compiled JavaScript bytecode.
//!
//! The VM keeps one operand stack shared by all calls. Each call gets a
//! frame with its own lexical scopes, receiver, and arguments; the frame
//! of the caller is saved while a callee runs. Top-level bindings live in the
//! outermost frame's first scope and are reachable from every frame.

use crate::ast::{BinaryOperator, UnaryOperator};
use crate::bytecode::{CallableKind, Chunk, FunctionTemplate, Instruction};
use crate::runtime::{ExecutionLimits, non_object, to_property_key};
use crate::{Function, JsError, JsErrorCategory, JsResult, Object, Value};
use std::sync::Arc;

/// Executes a [`Chunk`] using an owned operand stack.
#[derive(Debug, Default)]
pub struct Vm {
    stack: Vec<Value>,
    /// The running frame.
    frame: Frame,
    /// Suspended frames of the active callers, outermost first.
    callers: Vec<Frame>,
    steps_remaining: u64,
    loop_iterations_remaining: u64,
    call_depth_limit: usize,
}

/// Execution state of one chunk: the program or a single call.
#[derive(Debug, Default)]
struct Frame {
    chunk: Arc<Chunk>,
    /// Instruction to resume at; only meaningful while the frame is suspended.
    resume: usize,
    scopes: Vec<Vec<Slot>>,
    /// Height of the operand stack when the frame started.
    stack_base: usize,
    /// The receiver, or `None` in a derived constructor before `super(...)`.
    this: Option<Value>,
    arguments: Vec<Value>,
    /// The function being run; `None` for the program.
    callee: Option<Function>,
    /// The constructor `new` was applied to, for constructor frames.
    new_target: Option<Function>,
    mode: ReturnMode,
}

/// What the caller does with the value a finished frame produces.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum ReturnMode {
    /// Push the returned value.
    #[default]
    Call,
    /// Apply the constructor return rules, then push the result.
    Construct,
    /// Like `Construct`, and also bind the result as the caller's `this`.
    SuperCall,
}

#[derive(Debug, Clone)]
struct Slot {
    value: Option<Value>,
    kind: Option<crate::ast::VarKind>,
}

/// Which environment a binding instruction addresses.
#[derive(Debug, Clone, Copy)]
enum Target {
    /// The top-level scope of the program.
    Global,
    /// A scope of the running frame, counted outward from the innermost.
    Local(u16),
}

impl Vm {
    /// Creates a VM with an empty operand stack.
    pub fn new() -> Self {
        Self::default()
    }

    /// Executes `chunk` and returns its completion value.
    pub fn execute(&mut self, chunk: &Chunk) -> JsResult<Value> {
        self.execute_with_limits(chunk, ExecutionLimits::default())
    }

    /// Executes `chunk` with caller-provided instruction and loop limits.
    ///
    /// One step is charged for every instruction. A loop iteration is charged
    /// when execution takes a backward jump, which keeps an infinite loop
    /// bounded even when its condition and body are otherwise inexpensive.
    /// Nested calls beyond `limits.call_depth_limit` fail with a limit error.
    pub fn execute_with_limits(
        &mut self,
        chunk: &Chunk,
        limits: ExecutionLimits,
    ) -> JsResult<Value> {
        self.stack.clear();
        self.callers.clear();
        self.frame = Frame {
            chunk: Arc::new(chunk.clone()),
            this: Some(Value::Undefined),
            ..Frame::default()
        };
        self.steps_remaining = limits.step_limit;
        self.loop_iterations_remaining = limits.loop_iteration_limit;
        self.call_depth_limit = usize::try_from(limits.call_depth_limit).unwrap_or(usize::MAX);
        let result = self.run();
        // Drop references to script values held by an abandoned run.
        self.stack.clear();
        self.callers.clear();
        self.frame = Frame::default();
        result
    }

    fn run(&mut self) -> JsResult<Value> {
        let mut chunk = Arc::clone(&self.frame.chunk);
        let mut instruction_pointer = 0;
        while let Some(instruction) = chunk.code.get(instruction_pointer).copied() {
            if self.steps_remaining == 0 {
                return Err(JsError::with_context(
                    crate::JsErrorCategory::Limit,
                    "Script exceeded the evaluation step limit",
                    "bytecode instruction budget",
                ));
            }
            self.steps_remaining -= 1;
            instruction_pointer += 1;
            // Set when the instruction pushed or popped a frame.
            let mut switched = false;
            match instruction {
                Instruction::Constant(index) => self.push(
                    chunk
                        .constants
                        .get(usize::from(index))
                        .cloned()
                        .ok_or_else(|| JsError::runtime("bytecode constant index is invalid"))?,
                ),
                Instruction::Pop => {
                    self.pop()?;
                }
                Instruction::Swap => {
                    let right = self.pop()?;
                    let left = self.pop()?;
                    self.push(right);
                    self.push(left);
                }
                Instruction::Dup => {
                    let value = self.peek()?.clone();
                    self.push(value);
                }
                Instruction::Jump(target) => {
                    validate_jump(target, &chunk)?;
                    if target < instruction_pointer {
                        self.next_loop_iteration()?;
                    }
                    instruction_pointer = target;
                }
                Instruction::Return => {
                    if self.callers.is_empty() {
                        return Ok(self.stack.pop().unwrap_or(Value::Undefined));
                    }
                    self.finish_frame()?;
                    switched = true;
                }
                Instruction::Unary(operator) => {
                    let value = self.pop()?;
                    self.push(unary(operator, value)?);
                }
                Instruction::Binary(operator) => {
                    let right = self.pop()?;
                    let left = self.pop()?;
                    self.push(binary(operator, left, right)?);
                }
                Instruction::JumpIfFalse(target) => {
                    validate_jump(target, &chunk)?;
                    if !is_truthy(self.stack.last().ok_or_else(stack_underflow)?) {
                        instruction_pointer = target;
                    }
                }
                Instruction::JumpIfTrue(target) => {
                    validate_jump(target, &chunk)?;
                    if is_truthy(self.stack.last().ok_or_else(stack_underflow)?) {
                        instruction_pointer = target;
                    }
                }
                Instruction::LoadProperty(index) => {
                    let key = property_name(&chunk, index)?;
                    let target = self.pop()?;
                    self.push(load_property(&target, &key)?);
                }
                Instruction::LoadComputedProperty => {
                    let key = to_property_key(&self.pop()?);
                    let target = self.pop()?;
                    self.push(load_property(&target, &key)?);
                }
                Instruction::StoreProperty(index) => {
                    let key = property_name(&chunk, index)?;
                    let value = self.pop()?;
                    let target = self.pop()?;
                    store_property(&target, key, &value)?;
                    self.push(value);
                }
                Instruction::StoreComputedProperty => {
                    let value = self.pop()?;
                    let key = to_property_key(&self.pop()?);
                    let target = self.pop()?;
                    store_property(&target, key, &value)?;
                    self.push(value);
                }
                Instruction::LoadThis => {
                    let this = self.frame.this.clone().ok_or_else(|| {
                        JsError::new(
                            "Must call super constructor in derived class before accessing `this`",
                        )
                    })?;
                    self.push(this);
                }
                Instruction::LoadArgument(index) => {
                    let argument = self
                        .frame
                        .arguments
                        .get(usize::from(index))
                        .cloned()
                        .unwrap_or(Value::Undefined);
                    self.push(argument);
                }
                Instruction::MakeFunction(index) => {
                    let template = function_template(&chunk, index)?;
                    let function = Function::from_template(template, None, None, None);
                    self.push(Value::Function(function));
                }
                Instruction::Call(count) => {
                    let arguments = self.pop_arguments(count)?;
                    let callee = self.pop()?;
                    switched =
                        self.call(callee, Value::Undefined, arguments, instruction_pointer)?;
                }
                Instruction::CallMethod(count) => {
                    let arguments = self.pop_arguments(count)?;
                    let callee = self.pop()?;
                    let receiver = self.pop()?;
                    switched = self.call(callee, receiver, arguments, instruction_pointer)?;
                }
                Instruction::New(count) => {
                    let arguments = self.pop_arguments(count)?;
                    let constructor = self.pop()?;
                    let Value::Function(function) = constructor else {
                        return Err(not_a_constructor(&constructor));
                    };
                    switched = self.construct(
                        function.clone(),
                        function,
                        arguments,
                        ReturnMode::Construct,
                        instruction_pointer,
                    )?;
                }
                Instruction::SuperCall(count) => {
                    let arguments = self.pop_arguments(count)?;
                    let parent = self
                        .frame
                        .callee
                        .as_ref()
                        .and_then(Function::parent)
                        .cloned()
                        .ok_or_else(|| JsError::runtime("`super(...)` has no parent class"))?;
                    let new_target =
                        self.frame.new_target.clone().ok_or_else(|| {
                            JsError::runtime("`super(...)` outside a construction")
                        })?;
                    switched = self.construct(
                        new_target,
                        parent,
                        arguments,
                        ReturnMode::SuperCall,
                        instruction_pointer,
                    )?;
                }
                Instruction::LoadSuperProperty(index) => {
                    let key = property_name(&chunk, index)?;
                    let prototype = self
                        .frame
                        .callee
                        .as_ref()
                        .and_then(Function::home_object)
                        .ok_or_else(|| JsError::runtime("`super` has no home object"))?
                        .prototype();
                    let value = prototype.and_then(|prototype| prototype.get(&key));
                    self.push(value.ok_or_else(|| property_missing(&key))?);
                }
                Instruction::DefineClass {
                    function,
                    has_superclass,
                } => {
                    let parent = if has_superclass {
                        Some(self.pop_superclass()?)
                    } else {
                        None
                    };
                    let template = function_template(&chunk, function)?;
                    let parent_prototype = parent.as_ref().and_then(prototype_of);
                    let prototype = Object::with_prototype(parent_prototype);
                    let class = Function::from_template(
                        template,
                        Some(prototype.clone()),
                        parent,
                        Some(prototype),
                    );
                    self.push(Value::Function(class));
                }
                Instruction::DefineMethod { function, name } => {
                    let key = property_name(&chunk, name)?;
                    let template = function_template(&chunk, function)?;
                    let Some(Value::Function(class)) = self.stack.last() else {
                        return Err(JsError::runtime("method defined without a class"));
                    };
                    let prototype = prototype_of(class)
                        .ok_or_else(|| JsError::runtime("class has no prototype"))?;
                    let method =
                        Function::from_template(template, Some(prototype.clone()), None, None);
                    prototype.set(key, Value::Function(method));
                }
                Instruction::EnterScope(slot_count) => {
                    self.frame.scopes.push(vec![
                        Slot {
                            value: None,
                            kind: None
                        };
                        usize::from(slot_count)
                    ]);
                }
                Instruction::ExitScope => {
                    self.frame
                        .scopes
                        .pop()
                        .ok_or_else(|| JsError::runtime("bytecode scope underflow"))?;
                }
                Instruction::DeclareGlobal { slot, kind, name } => {
                    self.declare(Target::Global, slot, kind, name, &chunk)?;
                }
                Instruction::DeclareLocal { slot, kind, name } => {
                    self.declare(Target::Local(0), slot, kind, name, &chunk)?;
                }
                Instruction::LoadGlobal { slot, name } => {
                    let value = self.load(Target::Global, slot, name, &chunk)?;
                    self.push(value);
                }
                Instruction::LoadLocal { depth, slot, name } => {
                    let value = self.load(Target::Local(depth), slot, name, &chunk)?;
                    self.push(value);
                }
                Instruction::InitializeGlobal { slot, name } => {
                    let value = self.pop()?;
                    self.initialize(Target::Global, slot, value, name, &chunk)?;
                }
                Instruction::InitializeLocal { depth, slot, name } => {
                    let value = self.pop()?;
                    self.initialize(Target::Local(depth), slot, value, name, &chunk)?;
                }
                Instruction::StoreGlobal { slot, name } => {
                    let value = self.peek()?.clone();
                    self.store(Target::Global, slot, value, name, &chunk)?;
                }
                Instruction::StoreLocal { depth, slot, name } => {
                    let value = self.peek()?.clone();
                    self.store(Target::Local(depth), slot, value, name, &chunk)?;
                }
                Instruction::LoadUnresolved { name } => {
                    let name = self.binding_name(name, &chunk)?;
                    return Err(JsError::new(format!("`{name}` is not defined")));
                }
                Instruction::StoreUnresolved { name } => {
                    let name = self.binding_name(name, &chunk)?;
                    return Err(JsError::new(format!("`{name}` is not defined")));
                }
            }
            if switched {
                // The callee's first instruction, or the caller's next one.
                chunk = Arc::clone(&self.frame.chunk);
                instruction_pointer = self.frame.resume;
            }
        }
        Err(JsError::runtime(
            "bytecode ended without a return instruction",
        ))
    }

    fn push(&mut self, value: Value) {
        self.stack.push(value);
    }

    /// Pops `count` call arguments, returning them in call order.
    fn pop_arguments(&mut self, count: u16) -> JsResult<Vec<Value>> {
        let count = usize::from(count);
        let start = self
            .stack
            .len()
            .checked_sub(count)
            .ok_or_else(stack_underflow)?;
        Ok(self.stack.split_off(start))
    }

    /// Pops the value after `extends`, which must be a constructor.
    fn pop_superclass(&mut self) -> JsResult<Function> {
        match self.pop()? {
            Value::Function(function)
                if function
                    .template()
                    .is_some_and(|template| template.kind.is_constructor()) =>
            {
                Ok(function)
            }
            other => Err(JsError::new(format!(
                "Class extends value `{other}` is not a constructor"
            ))),
        }
    }

    /// Calls `callee` with `this` bound to `receiver`.
    ///
    /// Host functions finish immediately and push their result. A bytecode
    /// function gets a new frame; the return value tells the run loop to
    /// switch to it.
    fn call(
        &mut self,
        callee: Value,
        receiver: Value,
        arguments: Vec<Value>,
        resume: usize,
    ) -> JsResult<bool> {
        let Value::Function(function) = callee else {
            return Err(JsError::with_context(
                JsErrorCategory::Runtime,
                format!("`{callee}` is not a function"),
                "function call",
            ));
        };
        let Some(template) = function.template().cloned() else {
            return match function.call_host(&arguments) {
                Some(result) => {
                    self.push(result?);
                    Ok(false)
                }
                None => Err(JsError::runtime(
                    "the bytecode VM cannot call functions from the tree-walk evaluator",
                )),
            };
        };
        if template.kind.is_class() {
            return Err(JsError::with_context(
                JsErrorCategory::Runtime,
                format!(
                    "Class constructor `{}` cannot be invoked without `new`",
                    template.name
                ),
                "function call",
            ));
        }
        self.enter(
            Frame {
                chunk: Arc::clone(&template.chunk),
                this: Some(receiver),
                arguments,
                callee: Some(function),
                mode: ReturnMode::Call,
                ..Frame::default()
            },
            resume,
        )?;
        Ok(true)
    }

    /// Starts constructing with `function`, which is `new_target` for `new`
    /// and the parent class for `super(...)`.
    ///
    /// The instance is created here for base constructors and plain
    /// functions, with `new_target.prototype` as its prototype. A derived
    /// constructor starts without `this` and gets it from `super(...)`.
    fn construct(
        &mut self,
        new_target: Function,
        mut function: Function,
        arguments: Vec<Value>,
        mode: ReturnMode,
        resume: usize,
    ) -> JsResult<bool> {
        let mut template = function
            .template()
            .cloned()
            .ok_or_else(|| not_a_constructor(&Value::Function(function.clone())))?;
        // An implicit derived constructor just forwards to its parent.
        while template.kind == CallableKind::DefaultDerivedConstructor {
            function = function
                .parent()
                .cloned()
                .ok_or_else(|| JsError::runtime("derived class has no parent"))?;
            template = function
                .template()
                .cloned()
                .ok_or_else(|| JsError::runtime("parent class is not a bytecode function"))?;
        }
        if !template.kind.is_constructor() {
            return Err(not_a_constructor(&Value::Function(function)));
        }
        let this = match template.kind {
            CallableKind::DerivedConstructor => None,
            _ => Some(Value::Object(Object::with_prototype(prototype_of(
                &new_target,
            )))),
        };
        self.enter(
            Frame {
                chunk: Arc::clone(&template.chunk),
                this,
                arguments,
                callee: Some(function),
                new_target: Some(new_target),
                mode,
                ..Frame::default()
            },
            resume,
        )?;
        Ok(true)
    }

    /// Suspends the running frame, to resume at `resume`, and starts `frame`.
    fn enter(&mut self, mut frame: Frame, resume: usize) -> JsResult<()> {
        if self.callers.len() + 1 >= self.call_depth_limit {
            return Err(JsError::with_context(
                JsErrorCategory::Limit,
                "Maximum call stack size exceeded",
                "bytecode call depth",
            ));
        }
        frame.stack_base = self.stack.len();
        frame.resume = 0;
        self.frame.resume = resume;
        let caller = std::mem::replace(&mut self.frame, frame);
        self.callers.push(caller);
        Ok(())
    }

    /// Ends the running frame, delivering its result to the caller.
    ///
    /// Constructors follow JavaScript's return rule: returning an object
    /// replaces the new instance, while returning anything else yields the
    /// instance (`this`) instead.
    fn finish_frame(&mut self) -> JsResult<()> {
        let returned = if self.stack.len() > self.frame.stack_base {
            self.stack.pop().unwrap_or(Value::Undefined)
        } else {
            Value::Undefined
        };
        self.stack.truncate(self.frame.stack_base);
        let caller = self
            .callers
            .pop()
            .ok_or_else(|| JsError::runtime("bytecode call stack underflow"))?;
        let finished = std::mem::replace(&mut self.frame, caller);
        let result = match finished.mode {
            ReturnMode::Call => returned,
            ReturnMode::Construct | ReturnMode::SuperCall => {
                constructed_value(&finished, returned)?
            }
        };
        if finished.mode == ReturnMode::SuperCall {
            if self.frame.this.is_some() {
                return Err(JsError::new("Super constructor may only be called once"));
            }
            self.frame.this = Some(result.clone());
        }
        self.push(result);
        Ok(())
    }

    fn next_loop_iteration(&mut self) -> JsResult<()> {
        if self.loop_iterations_remaining == 0 {
            return Err(JsError::with_context(
                crate::JsErrorCategory::Limit,
                "Script exceeded the loop iteration limit",
                "bytecode loop iteration",
            ));
        }
        self.loop_iterations_remaining -= 1;
        Ok(())
    }

    fn pop(&mut self) -> JsResult<Value> {
        self.stack.pop().ok_or_else(stack_underflow)
    }

    fn peek(&self) -> JsResult<&Value> {
        self.stack.last().ok_or_else(stack_underflow)
    }

    fn slot_mut(&mut self, target: Target, slot: u16) -> JsResult<&mut Slot> {
        let scopes = match target {
            Target::Global => match self.callers.first_mut() {
                Some(program) => &mut program.scopes,
                None => &mut self.frame.scopes,
            },
            Target::Local(_) => &mut self.frame.scopes,
        };
        let index = scope_index(scopes.len(), target)?;
        scopes
            .get_mut(index)
            .and_then(|scope| scope.get_mut(usize::from(slot)))
            .ok_or_else(|| JsError::runtime("bytecode binding slot is invalid"))
    }

    fn slot(&self, target: Target, slot: u16) -> JsResult<&Slot> {
        let scopes = match target {
            Target::Global => match self.callers.first() {
                Some(program) => &program.scopes,
                None => &self.frame.scopes,
            },
            Target::Local(_) => &self.frame.scopes,
        };
        let index = scope_index(scopes.len(), target)?;
        scopes
            .get(index)
            .and_then(|scope| scope.get(usize::from(slot)))
            .ok_or_else(|| JsError::runtime("bytecode binding slot is invalid"))
    }

    fn binding_name<'a>(&self, name: u16, chunk: &'a Chunk) -> JsResult<&'a str> {
        chunk
            .names
            .get(usize::from(name))
            .map(String::as_str)
            .ok_or_else(|| JsError::runtime("bytecode binding name is invalid"))
    }

    fn declare(
        &mut self,
        target: Target,
        slot: u16,
        kind: crate::ast::VarKind,
        name: u16,
        chunk: &Chunk,
    ) -> JsResult<()> {
        let name = self.binding_name(name, chunk)?.to_owned();
        let entry = self.slot_mut(target, slot)?;
        entry.kind = Some(kind);
        if kind == crate::ast::VarKind::Var {
            entry.value = Some(Value::Undefined);
        }
        let _ = name;
        Ok(())
    }

    fn load(&self, target: Target, slot: u16, name: u16, chunk: &Chunk) -> JsResult<Value> {
        let name = self.binding_name(name, chunk)?;
        self.slot(target, slot)?
            .value
            .clone()
            .ok_or_else(|| JsError::new(format!("Cannot access `{name}` before initialization")))
    }

    fn initialize(
        &mut self,
        target: Target,
        slot: u16,
        value: Value,
        name: u16,
        chunk: &Chunk,
    ) -> JsResult<()> {
        let name = self.binding_name(name, chunk)?.to_owned();
        let entry = self.slot_mut(target, slot)?;
        if entry.value.is_some() && entry.kind == Some(crate::ast::VarKind::Const) {
            return Err(JsError::new(format!("Assignment to constant `{name}`")));
        }
        entry.value = Some(value);
        Ok(())
    }

    fn store(
        &mut self,
        target: Target,
        slot: u16,
        value: Value,
        name: u16,
        chunk: &Chunk,
    ) -> JsResult<()> {
        let name = self.binding_name(name, chunk)?.to_owned();
        let entry = self.slot_mut(target, slot)?;
        if entry.value.is_none() {
            return Err(JsError::new(format!(
                "Cannot access `{name}` before initialization"
            )));
        }
        if entry.kind == Some(crate::ast::VarKind::Const) {
            return Err(JsError::new(format!("Assignment to constant `{name}`")));
        }
        entry.value = Some(value);
        Ok(())
    }
}

/// Position in a frame's scope list of the scope `target` addresses.
fn scope_index(scope_count: usize, target: Target) -> JsResult<usize> {
    match target {
        Target::Global => {
            if scope_count == 0 {
                Err(JsError::runtime("bytecode global scope is unavailable"))
            } else {
                Ok(0)
            }
        }
        Target::Local(depth) => scope_count
            .checked_sub(1 + usize::from(depth))
            .ok_or_else(|| JsError::runtime("bytecode scope depth is invalid")),
    }
}

fn function_template(chunk: &Chunk, index: u16) -> JsResult<Arc<FunctionTemplate>> {
    chunk
        .functions
        .get(usize::from(index))
        .cloned()
        .ok_or_else(|| JsError::runtime("bytecode function index is invalid"))
}

/// The object a constructor's instances inherit from, if it has one.
fn prototype_of(constructor: &Function) -> Option<Object> {
    match constructor.get_own("prototype") {
        Some(Value::Object(prototype)) => Some(prototype),
        _ => None,
    }
}

fn not_a_constructor(value: &Value) -> JsError {
    JsError::with_context(
        JsErrorCategory::Runtime,
        format!("`{value}` is not a constructor"),
        "new expression",
    )
}

/// Applies the constructor return rules to a finished construction frame.
fn constructed_value(frame: &Frame, returned: Value) -> JsResult<Value> {
    if matches!(returned, Value::Object(_) | Value::Function(_)) {
        return Ok(returned);
    }
    let derived = frame
        .callee
        .as_ref()
        .and_then(Function::template)
        .is_some_and(|template| template.kind == CallableKind::DerivedConstructor);
    if derived && returned != Value::Undefined {
        return Err(JsError::new(
            "Derived constructors may only return an object or undefined",
        ));
    }
    frame.this.clone().ok_or_else(|| {
        JsError::new("Must call super constructor in derived class before returning from derived constructor")
    })
}

fn validate_jump(target: usize, chunk: &Chunk) -> JsResult<()> {
    if target > chunk.code.len() {
        return Err(JsError::runtime("bytecode jump target is invalid"));
    }
    Ok(())
}

fn stack_underflow() -> JsError {
    JsError::runtime("bytecode operand stack underflow")
}

/// Reads the string constant naming a property.
fn property_name(chunk: &Chunk, index: u16) -> JsResult<String> {
    match chunk.constants.get(usize::from(index)) {
        Some(Value::String(name)) => Ok(name.clone()),
        _ => Err(JsError::runtime(
            "bytecode property name constant is invalid",
        )),
    }
}

fn property_missing(key: &str) -> JsError {
    JsError::with_context(
        JsErrorCategory::Runtime,
        format!("Property `{key}` is not defined"),
        "property access",
    )
}

/// Reads a property of an object, following its prototype chain, or an own
/// property of a bytecode function such as a class's `prototype`.
fn load_property(target: &Value, key: &str) -> JsResult<Value> {
    match target {
        Value::Object(object) => object.get(key),
        Value::Function(function) if function.template().is_some() => function.get_own(key),
        _ => return Err(non_object(target, "read", key, "property access")),
    }
    .ok_or_else(|| property_missing(key))
}

fn store_property(target: &Value, key: String, value: &Value) -> JsResult<()> {
    match target {
        Value::Object(object) => object.set(key, value.clone()),
        Value::Function(function) if function.set_own(&key, value.clone()) => {}
        _ => return Err(non_object(target, "set", &key, "property assignment")),
    }
    Ok(())
}

fn unary(operator: UnaryOperator, value: Value) -> JsResult<Value> {
    match operator {
        UnaryOperator::Negate => Ok(Value::Number(-to_number(&value)?)),
        UnaryOperator::Not => Ok(Value::Boolean(!is_truthy(&value))),
        UnaryOperator::BitwiseNot => Ok(Value::Number(f64::from(!to_int32(&value)?))),
    }
}

fn binary(operator: BinaryOperator, left: Value, right: Value) -> JsResult<Value> {
    match operator {
        BinaryOperator::Add => match (left, right) {
            (Value::String(left), right) => Ok(Value::String(left + &right.to_string())),
            (left, Value::String(right)) => Ok(Value::String(left.to_string() + &right)),
            (left, right) => Ok(Value::Number(to_number(&left)? + to_number(&right)?)),
        },
        BinaryOperator::Subtract => number_binary(left, right, |a, b| a - b),
        BinaryOperator::Multiply => number_binary(left, right, |a, b| a * b),
        BinaryOperator::Divide => number_binary(left, right, |a, b| a / b),
        BinaryOperator::Remainder => number_binary(left, right, |a, b| a % b),
        BinaryOperator::BitwiseAnd => int_binary(left, right, |a, b| a & b),
        BinaryOperator::BitwiseOr => int_binary(left, right, |a, b| a | b),
        BinaryOperator::BitwiseXor => int_binary(left, right, |a, b| a ^ b),
        BinaryOperator::LeftShift => {
            int_binary(left, right, |a, b| a.wrapping_shl((b as u32) & 31))
        }
        BinaryOperator::SignedRightShift => int_binary(left, right, |a, b| a >> ((b as u32) & 31)),
        BinaryOperator::UnsignedRightShift => Ok(Value::Number(f64::from(
            (to_int32(&left)? as u32).wrapping_shr((to_uint32(&right)? as u32) & 31),
        ))),
        BinaryOperator::Less => Ok(Value::Boolean(to_number(&left)? < to_number(&right)?)),
        BinaryOperator::LessEqual => Ok(Value::Boolean(to_number(&left)? <= to_number(&right)?)),
        BinaryOperator::Greater => Ok(Value::Boolean(to_number(&left)? > to_number(&right)?)),
        BinaryOperator::GreaterEqual => Ok(Value::Boolean(to_number(&left)? >= to_number(&right)?)),
        BinaryOperator::StrictEqual => Ok(Value::Boolean(left == right)),
        BinaryOperator::StrictNotEqual => Ok(Value::Boolean(left != right)),
        BinaryOperator::Equal => Ok(Value::Boolean(left == right)),
        BinaryOperator::NotEqual => Ok(Value::Boolean(left != right)),
    }
}

fn number_binary(
    left: Value,
    right: Value,
    operation: impl FnOnce(f64, f64) -> f64,
) -> JsResult<Value> {
    Ok(Value::Number(operation(
        to_number(&left)?,
        to_number(&right)?,
    )))
}

fn int_binary(
    left: Value,
    right: Value,
    operation: impl FnOnce(i32, i32) -> i32,
) -> JsResult<Value> {
    Ok(Value::Number(f64::from(operation(
        to_int32(&left)?,
        to_int32(&right)?,
    ))))
}

fn to_number(value: &Value) -> JsResult<f64> {
    match value {
        Value::Number(value) => Ok(*value),
        Value::Boolean(value) => Ok(f64::from(*value as u8)),
        Value::Null => Ok(0.0),
        Value::Undefined => Ok(f64::NAN),
        Value::String(value) if value.trim().is_empty() => Ok(0.0),
        Value::String(value) => Ok(value.trim().parse().unwrap_or(f64::NAN)),
        Value::Function(_) | Value::Object(_) => {
            Err(JsError::runtime("cannot convert object to number"))
        }
    }
}

fn to_int32(value: &Value) -> JsResult<i32> {
    Ok(to_number(value)? as i32)
}

fn to_uint32(value: &Value) -> JsResult<u32> {
    Ok(to_number(value)? as u32)
}

fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Undefined | Value::Null => false,
        Value::Boolean(value) => *value,
        Value::Number(value) => *value != 0.0 && !value.is_nan(),
        Value::String(value) => !value.is_empty(),
        Value::Function(_) | Value::Object(_) => true,
    }
}
