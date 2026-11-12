//! Stack-based virtual machine for executing compiled JavaScript bytecode.
//!
//! The VM keeps one operand stack shared by all calls. Each call gets a
//! frame with its own lexical scopes, receiver, and arguments; the frame
//! of the caller is saved while a callee runs. Top-level bindings live in the
//! outermost frame's first scope and are reachable from every frame.

use crate::bytecode::{CallableKind, Chunk, FunctionTemplate, Instruction};
use crate::runtime::{
    BytecodeCapture, ExecutionLimits, array_len, binary, copy_data_properties, delete_property,
    get_array_property, is_nullish, is_truthy, non_object, set_array_property, spread_values,
    to_property_key, unary,
};
use crate::{Function, JsError, JsErrorCategory, JsResult, Object, Value};
use std::sync::{Arc, Mutex};

/// Executes a [`Chunk`] using an owned operand stack.
#[derive(Debug, Default)]
pub struct Vm {
    stack: Vec<Value>,
    /// The running frame.
    frame: Frame,
    /// Suspended frames of the active callers, outermost first.
    callers: Vec<Frame>,
    /// Active exception handlers, innermost last. Each was registered by an
    /// `EnterTry` of the frame at `frame_depth` and has not been left yet.
    handlers: Vec<Handler>,
    steps_remaining: u64,
    loop_iterations_remaining: u64,
    call_depth_limit: usize,
}

/// An active exception handler registered by [`Instruction::EnterTry`].
#[derive(Debug, Clone, Copy)]
struct Handler {
    /// Instruction offset, in the registering frame's chunk, that receives
    /// the thrown value.
    target: usize,
    /// Number of suspended callers when the handler was registered, which
    /// identifies the registering frame.
    frame_depth: usize,
    /// Operand stack height to restore before delivering the exception.
    stack_height: usize,
    /// Lexical scope depth of the registering frame to restore.
    scope_depth: usize,
}

/// Execution state of one chunk: the program or a single call.
#[derive(Debug, Default)]
struct Frame {
    chunk: Arc<Chunk>,
    /// Instruction to resume at; only meaningful while the frame is suspended.
    resume: usize,
    scopes: Vec<Vec<Slot>>,
    captures: Vec<BytecodeCapture>,
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
    value: Arc<Mutex<Option<Value>>>,
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

    /// Constructs an instance by running a bytecode function with `new`.
    ///
    /// The function's captured realm globals are not available to this
    /// standalone construction path; constructors that read globals may
    /// therefore fail. This is intended for host APIs that need to invoke a
    /// constructor without an active script VM.
    pub fn construct_function(function: Function, arguments: &[Value]) -> JsResult<Value> {
        let argument_count = u16::try_from(arguments.len())
            .map_err(|_| JsError::runtime("too many constructor arguments"))?;
        let mut constants = Vec::with_capacity(arguments.len() + 1);
        constants.push(Value::Function(function));
        constants.extend_from_slice(arguments);
        let mut code = Vec::with_capacity(arguments.len() + 3);
        code.push(Instruction::Constant(0));
        for index in 0..arguments.len() {
            let index = u16::try_from(index + 1)
                .map_err(|_| JsError::runtime("too many constructor arguments"))?;
            code.push(Instruction::Constant(index));
        }
        code.push(Instruction::New(argument_count));
        code.push(Instruction::Return);
        let chunk = Chunk {
            locations: vec![None; code.len()],
            code,
            constants,
            names: Vec::new(),
            functions: Vec::new(),
        };
        Self::new().execute(&chunk)
    }

    /// Calls `function` with an explicit receiver and arguments.
    ///
    /// Bytecode functions execute in a fresh VM without their original global
    /// environment. This host entry point supports callbacks that only depend
    /// on their receiver and arguments.
    pub fn call_function(
        function: Function,
        receiver: Value,
        arguments: &[Value],
    ) -> JsResult<Value> {
        let argument_count = u16::try_from(arguments.len())
            .map_err(|_| JsError::runtime("too many function arguments"))?;
        let mut constants = Vec::with_capacity(arguments.len() + 2);
        constants.push(receiver);
        constants.push(Value::Function(function));
        constants.extend_from_slice(arguments);
        let mut code = Vec::with_capacity(arguments.len() + 4);
        code.push(Instruction::Constant(0));
        code.push(Instruction::Constant(1));
        for index in 0..arguments.len() {
            let index = u16::try_from(index + 2)
                .map_err(|_| JsError::runtime("too many function arguments"))?;
            code.push(Instruction::Constant(index));
        }
        code.push(Instruction::CallMethod(argument_count));
        code.push(Instruction::Return);
        let chunk = Chunk {
            locations: vec![None; code.len()],
            code,
            constants,
            names: Vec::new(),
            functions: Vec::new(),
        };
        Self::new().execute(&chunk)
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
        log::debug!(
            "executing chunk ({} instructions, {} functions)",
            chunk.code.len(),
            chunk.functions.len()
        );
        self.stack.clear();
        self.callers.clear();
        self.handlers.clear();
        self.frame = Frame {
            chunk: Arc::new(chunk.clone()),
            this: Some(Value::Undefined),
            ..Frame::default()
        };
        self.steps_remaining = limits.step_limit;
        self.loop_iterations_remaining = limits.loop_iteration_limit;
        self.call_depth_limit = usize::try_from(limits.call_depth_limit).unwrap_or(usize::MAX);
        let result = self.run();
        match &result {
            Ok(_) => log::debug!("chunk finished with {} steps left", self.steps_remaining),
            Err(error) => log::debug!("chunk failed ({:?}): {}", error.category, error.message),
        }
        // Drop references to script values held by an abandoned run.
        self.stack.clear();
        self.callers.clear();
        self.handlers.clear();
        self.frame = Frame::default();
        result
    }

    /// Runs until the program returns or an error nothing handles.
    ///
    /// Exceptions are resolved here, entirely inside the VM: an error raised
    /// by an instruction, a callee, or a host function is delivered to the
    /// innermost active handler, and execution resumes at its target.
    fn run(&mut self) -> JsResult<Value> {
        let mut start = 0;
        loop {
            match self.run_from(start) {
                Ok(value) => return Ok(value),
                Err(error) => start = self.unwind(error)?,
            }
        }
    }

    /// Delivers `error` to the innermost handler and returns the offset to
    /// resume at in the (possibly different) running frame.
    ///
    /// Callee frames above the handler's frame are discarded without
    /// finishing, and the operand stack and scope depth return to what the
    /// handler recorded. Errors that scripts cannot catch, or any error with
    /// no handler, are returned unchanged as the script's failure.
    fn unwind(&mut self, error: JsError) -> JsResult<usize> {
        if !error.category.is_catchable() {
            return Err(error);
        }
        let Some(handler) = self.handlers.pop() else {
            return Err(error);
        };
        log::debug!(
            "unwinding {} frame(s) to a handler: {}",
            self.callers.len().saturating_sub(handler.frame_depth),
            error.message
        );
        while self.callers.len() > handler.frame_depth {
            self.frame = self
                .callers
                .pop()
                .ok_or_else(|| JsError::runtime("bytecode call stack underflow"))?;
        }
        self.stack.truncate(handler.stack_height);
        self.frame.scopes.truncate(handler.scope_depth);
        self.push(error.into_caught_value());
        Ok(handler.target)
    }

    fn run_from(&mut self, start: usize) -> JsResult<Value> {
        let mut chunk = Arc::clone(&self.frame.chunk);
        let mut instruction_pointer = start;
        while let Some(instruction) = chunk.code.get(instruction_pointer).copied() {
            if self.steps_remaining == 0 {
                log::warn!("instruction step limit reached in the bytecode VM");
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
                        .ok_or_else(|| {
                            bytecode_error(
                                &chunk,
                                instruction_pointer - 1,
                                "bytecode constant index is invalid",
                            )
                        })?,
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
                    validate_jump(target, &chunk, instruction_pointer - 1)?;
                    if target < instruction_pointer {
                        self.next_loop_iteration()?;
                    }
                    instruction_pointer = target;
                }
                Instruction::Throw => {
                    let value = self.pop()?;
                    return Err(JsError::rethrown(value));
                }
                Instruction::EnterTry(target) => {
                    validate_jump(target, &chunk, instruction_pointer - 1)?;
                    self.handlers.push(Handler {
                        target,
                        frame_depth: self.callers.len(),
                        stack_height: self.stack.len(),
                        scope_depth: self.frame.scopes.len(),
                    });
                }
                Instruction::ExitTry => match self.handlers.pop() {
                    Some(handler) if handler.frame_depth == self.callers.len() => {}
                    _ => {
                        return Err(bytecode_error(
                            &chunk,
                            instruction_pointer - 1,
                            "bytecode exited an exception handler it does not own",
                        ));
                    }
                },
                Instruction::Return => {
                    if self.callers.is_empty() {
                        return Ok(self.stack.pop().unwrap_or(Value::Undefined));
                    }
                    self.finish_frame()?;
                    switched = true;
                }
                Instruction::Unary(operator) => {
                    let value = self.pop()?;
                    self.push(unary(operator, &value));
                }
                Instruction::Binary(operator) => {
                    let right = self.pop()?;
                    let left = self.pop()?;
                    self.push(binary(operator, &left, &right)?);
                }
                Instruction::Dup2 => {
                    let length = self.stack.len();
                    if length < 2 {
                        return Err(stack_underflow());
                    }
                    let (first, second) = (
                        self.stack[length - 2].clone(),
                        self.stack[length - 1].clone(),
                    );
                    self.push(first);
                    self.push(second);
                }
                Instruction::MoveTop(passed) => {
                    let value = self.pop()?;
                    let at = self
                        .stack
                        .len()
                        .checked_sub(usize::from(passed))
                        .ok_or_else(stack_underflow)?;
                    self.stack.insert(at, value);
                }
                Instruction::JumpIfNullish(target) => {
                    validate_jump(target, &chunk, instruction_pointer - 1)?;
                    if is_nullish(self.stack.last().ok_or_else(stack_underflow)?) {
                        instruction_pointer = target;
                    }
                }
                Instruction::DeleteProperty(index) => {
                    let key = property_name(&chunk, index)?;
                    let target = self.pop()?;
                    self.push(Value::Boolean(delete_property(&target, &key)?));
                }
                Instruction::DeleteComputedProperty => {
                    let key = to_property_key(&self.pop()?);
                    let target = self.pop()?;
                    self.push(Value::Boolean(delete_property(&target, &key)?));
                }
                Instruction::ArrayPush => {
                    let value = self.pop()?;
                    append_to_array(self.peek()?, value)?;
                }
                Instruction::ArrayExtend => {
                    let source = self.pop()?;
                    let values = spread_values(&source)?;
                    let array = self.peek()?.clone();
                    for value in values {
                        append_to_array(&array, value)?;
                    }
                }
                Instruction::SpreadProperties => {
                    let source = self.pop()?;
                    let Value::Object(target) = self.peek()? else {
                        return Err(JsError::runtime(
                            "bytecode object spread target is not an object",
                        ));
                    };
                    copy_data_properties(target, &source)?;
                }
                Instruction::CallSpread => {
                    let arguments = self.pop_spread_arguments()?;
                    let callee = self.pop()?;
                    switched =
                        self.call(callee, Value::Undefined, arguments, instruction_pointer)?;
                }
                Instruction::CallMethodSpread => {
                    let arguments = self.pop_spread_arguments()?;
                    let callee = self.pop()?;
                    let receiver = self.pop()?;
                    switched = self.call(callee, receiver, arguments, instruction_pointer)?;
                }
                Instruction::NewSpread => {
                    let arguments = self.pop_spread_arguments()?;
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
                Instruction::JumpIfFalse(target) => {
                    validate_jump(target, &chunk, instruction_pointer - 1)?;
                    if !is_truthy(self.stack.last().ok_or_else(stack_underflow)?) {
                        instruction_pointer = target;
                    }
                }
                Instruction::JumpIfTrue(target) => {
                    validate_jump(target, &chunk, instruction_pointer - 1)?;
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
                Instruction::LoadCapture { index, name } => {
                    let name = self.binding_name(name, &chunk)?.to_owned();
                    let capture = self
                        .frame
                        .captures
                        .get(usize::from(index))
                        .ok_or_else(|| JsError::runtime("bytecode capture is invalid"))?;
                    let value = capture.value.lock().unwrap().clone().ok_or_else(|| {
                        JsError::new(format!("Cannot access `{name}` before initialization"))
                    })?;
                    self.push(value);
                }
                Instruction::StoreCapture { index, name } => {
                    let name = self.binding_name(name, &chunk)?.to_owned();
                    let value = self.peek()?.clone();
                    let capture = self
                        .frame
                        .captures
                        .get(usize::from(index))
                        .ok_or_else(|| JsError::runtime("bytecode capture is invalid"))?;
                    if capture.value.lock().unwrap().is_none() {
                        return Err(JsError::new(format!(
                            "Cannot access `{name}` before initialization"
                        )));
                    }
                    if capture.kind == Some(crate::ast::VarKind::Const) {
                        return Err(JsError::new(format!("Assignment to constant `{name}`")));
                    }
                    *capture.value.lock().unwrap() = Some(value);
                }
                Instruction::MakeFunction(index) => {
                    let template = function_template(&chunk, index)?;
                    let mut captures = Vec::with_capacity(template.captures.len());
                    for source in &template.captures {
                        let capture = match source {
                            crate::bytecode::CaptureSource::Local { depth, slot } => {
                                let scope =
                                    scope_index(self.frame.scopes.len(), Target::Local(*depth))?;
                                let slot = self.frame.scopes[scope]
                                    .get(usize::from(*slot))
                                    .ok_or_else(|| JsError::runtime(format!("bytecode capture slot is invalid (depth {depth}, slot {slot}, scopes {})", self.frame.scopes.len())))?;
                                BytecodeCapture {
                                    value: Arc::clone(&slot.value),
                                    kind: slot.kind,
                                }
                            }
                            crate::bytecode::CaptureSource::Capture { index } => self
                                .frame
                                .captures
                                .get(usize::from(*index))
                                .cloned()
                                .ok_or_else(|| JsError::runtime("bytecode capture is invalid"))?,
                        };
                        captures.push(capture);
                    }
                    let lexical_this = (template.kind == CallableKind::Arrow)
                        .then(|| self.frame.this.clone().unwrap_or(Value::Undefined));
                    let function = Function::from_template_with_captures(
                        template,
                        None,
                        None,
                        None,
                        captures,
                        lexical_this,
                    );
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
                    let value = prototype
                        .and_then(|prototype| prototype.get(&key))
                        .unwrap_or(Value::Undefined);
                    self.push(value);
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
                Instruction::NewObject => self.push(Value::Object(Object::new())),
                Instruction::NewArray(count) => {
                    let values = self.pop_arguments(count)?;
                    let array = Object::new();
                    for (index, value) in values.into_iter().enumerate() {
                        array.set(index.to_string(), value);
                    }
                    array.set("length", Value::Number(f64::from(count)));
                    self.push(Value::Array(array));
                }
                Instruction::DefineProperty(index) => {
                    let key = property_name(&chunk, index)?;
                    let value = self.pop()?;
                    store_property(self.peek()?, key, &value)?;
                }
                Instruction::DefineComputedProperty => {
                    let value = self.pop()?;
                    let key = to_property_key(&self.pop()?);
                    store_property(self.peek()?, key, &value)?;
                }
                Instruction::EnterScope(slot_count) => {
                    self.frame.scopes.push(
                        (0..usize::from(slot_count))
                            .map(|_| Slot {
                                value: Arc::new(Mutex::new(None)),
                                kind: None,
                            })
                            .collect(),
                    );
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
        log::error!("bytecode ended without a return instruction");
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

    /// Pops the array that holds the arguments of a spread call, returning
    /// them in call order.
    fn pop_spread_arguments(&mut self) -> JsResult<Vec<Value>> {
        match self.pop()? {
            array @ Value::Array(_) => spread_values(&array),
            _ => Err(JsError::runtime(
                "bytecode spread arguments are not an array",
            )),
        }
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
            log::debug!("call target is not a function");
            return Err(JsError::with_context(
                JsErrorCategory::Runtime,
                format!("`{callee}` is not a function"),
                "function call",
            ));
        };
        let Some(template) = function.template().cloned() else {
            log::trace!("calling host function with {} arguments", arguments.len());
            return match function.call_host(&arguments) {
                Some(result) => {
                    self.push(result?);
                    Ok(false)
                }
                None => {
                    log::warn!("bytecode VM cannot call a tree-walk function");
                    Err(JsError::runtime(
                        "the bytecode VM cannot call functions from the tree-walk evaluator",
                    ))
                }
            };
        };
        log::trace!(
            "calling `{}` with {} arguments at depth {}",
            template.name,
            arguments.len(),
            self.callers.len() + 1
        );
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
        let call_receiver = if template.kind == CallableKind::Arrow {
            function.lexical_this().unwrap_or(Value::Undefined)
        } else {
            receiver
        };
        let captures = function
            .captures()
            .map(|captures| captures.as_ref().clone())
            .unwrap_or_default();
        self.enter(
            Frame {
                chunk: Arc::clone(&template.chunk),
                this: Some(call_receiver),
                arguments,
                callee: Some(function),
                captures,
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
        let captures = function
            .captures()
            .map(|captures| captures.as_ref().clone())
            .unwrap_or_default();
        self.enter(
            Frame {
                chunk: Arc::clone(&template.chunk),
                this,
                arguments,
                callee: Some(function),
                new_target: Some(new_target),
                captures,
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
            log::warn!(
                "call depth limit of {} reached in the bytecode VM",
                self.call_depth_limit
            );
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
        // Handlers of the finished frame cannot outlive it.
        while self
            .handlers
            .last()
            .is_some_and(|handler| handler.frame_depth > self.callers.len())
        {
            self.handlers.pop();
        }
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
            log::warn!("loop iteration limit reached in the bytecode VM");
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
            .ok_or_else(|| JsError::runtime("bytecode binding slot is invalid (read)"))
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
            *entry.value.lock().unwrap() = Some(Value::Undefined);
        }
        let _ = name;
        Ok(())
    }

    fn load(&self, target: Target, slot: u16, name: u16, chunk: &Chunk) -> JsResult<Value> {
        let name = self.binding_name(name, chunk)?;
        self.slot(target, slot)?
            .value
            .lock()
            .unwrap()
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
        if entry.value.lock().unwrap().is_some() && entry.kind == Some(crate::ast::VarKind::Const) {
            return Err(JsError::new(format!("Assignment to constant `{name}`")));
        }
        *entry.value.lock().unwrap() = Some(value);
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
        if entry.value.lock().unwrap().is_none() {
            return Err(JsError::new(format!(
                "Cannot access `{name}` before initialization"
            )));
        }
        if entry.kind == Some(crate::ast::VarKind::Const) {
            return Err(JsError::new(format!("Assignment to constant `{name}`")));
        }
        *entry.value.lock().unwrap() = Some(value);
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
        Some(Value::Object(prototype) | Value::Array(prototype)) => Some(prototype),
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
    if matches!(
        returned,
        Value::Object(_) | Value::Array(_) | Value::Function(_)
    ) {
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

fn validate_jump(target: usize, chunk: &Chunk, offset: usize) -> JsResult<()> {
    if target > chunk.code.len() {
        return Err(bytecode_error(
            chunk,
            offset,
            "bytecode jump target is invalid",
        ));
    }
    Ok(())
}

fn stack_underflow() -> JsError {
    log::error!("bytecode operand stack underflow");
    JsError::runtime("bytecode operand stack underflow")
}

fn bytecode_error(chunk: &Chunk, offset: usize, message: &'static str) -> JsError {
    log::error!("invalid bytecode at offset {offset}: {message}");
    let error = JsError::runtime(message);
    chunk
        .location(offset)
        .map_or(error.clone(), |location| error.with_location(location))
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

/// Reads a property of an object, following its prototype chain, or an own
/// property of a bytecode function such as a class's `prototype`.
///
/// A missing property reads as `undefined`. Reading a property of any other
/// value is an error.
fn load_property(target: &Value, key: &str) -> JsResult<Value> {
    match target {
        Value::Object(object) => Ok(object.get_property(key)?.unwrap_or(Value::Undefined)),
        Value::Array(array) => Ok(get_array_property(array, key)),
        Value::Function(function) if function.template().is_some() => {
            Ok(function.get_own(key).unwrap_or(Value::Undefined))
        }
        _ => Err(non_object(target, "read", key, "property access")),
    }
}

/// Appends `value` to the array literal being built, which must be an array.
fn append_to_array(array: &Value, value: Value) -> JsResult<()> {
    let Value::Array(array) = array else {
        return Err(JsError::runtime(
            "bytecode array append target is not an array",
        ));
    };
    let length = array_len(array);
    set_array_property(array, &length.to_string(), value)
}

fn store_property(target: &Value, key: String, value: &Value) -> JsResult<()> {
    match target {
        Value::Object(object) => object.set_property(key, value.clone())?,
        Value::Array(array) => set_array_property(array, &key, value.clone())?,
        Value::Function(function) if function.set_own(&key, value.clone()) => {}
        _ => return Err(non_object(target, "set", &key, "property assignment")),
    }
    Ok(())
}
