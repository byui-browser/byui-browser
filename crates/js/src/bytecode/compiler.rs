//! AST-to-bytecode compiler.

use super::{CallableKind, Chunk, FunctionTemplate, Instruction};
use crate::Value;
use crate::ast::{
    ClassDefinition, Expr, LogicalOperator, MemberProperty, Program, PropertyKey, Statement,
    VarKind,
};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

#[path = "control_flow.rs"]
mod control_flow;

/// A source construct that the initial bytecode compiler cannot lower yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    /// Human-readable description of the unsupported construct.
    pub message: String,
    /// Source location of the unsupported construct, when the compiler has one.
    pub location: Option<super::SourceLocation>,
}

impl CompileError {
    fn unsupported(construct: &str) -> Self {
        Self {
            message: format!("bytecode compiler does not support {construct} yet"),
            location: None,
        }
    }

    /// Attaches a source location to this compilation failure.
    pub fn with_location(mut self, location: super::SourceLocation) -> Self {
        self.location = Some(location);
        self
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)?;
        if let Some(location) = self.location {
            write!(
                formatter,
                " at line {}, column {}",
                location.line, location.column
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for CompileError {}

/// Compiles supported expressions and variable bindings into a fresh [`Chunk`].
#[derive(Debug)]
pub struct Compiler {
    chunk: Chunk,
    scopes: Vec<Scope>,
    /// `None` for the top-level program, otherwise the kind of function being
    /// compiled. Scope 0 is the global scope only when this is `None`.
    function: Option<CallableKind>,
    /// Top-level bindings, visible from every function body.
    globals: HashMap<String, Binding>,
    /// Names of local variables of enclosing blocks and functions. A function
    /// body cannot capture them, so referring to one is a compile error.
    enclosing: HashSet<String>,
}

#[derive(Debug)]
struct Scope {
    bindings: HashMap<String, Binding>,
    slots: u16,
}

#[derive(Debug, Clone, Copy)]
struct Binding {
    scope: usize,
    slot: u16,
    kind: VarKind,
    global: bool,
}

impl Compiler {
    /// Creates a compiler with an empty constant pool and instruction stream.
    pub fn new() -> Self {
        Self {
            chunk: Chunk::new(),
            scopes: vec![Scope {
                bindings: HashMap::new(),
                slots: 0,
            }],
            function: None,
            globals: HashMap::new(),
            enclosing: HashSet::new(),
        }
    }

    /// Creates the compiler for a function body nested in this compiler's code.
    fn child(&self, kind: CallableKind) -> Self {
        let mut enclosing = self.enclosing.clone();
        for (index, scope) in self.scopes.iter().enumerate() {
            if !self.is_global_scope(index) {
                enclosing.extend(scope.bindings.keys().cloned());
            }
        }
        let globals = if self.function.is_none() {
            self.scopes[0].bindings.clone()
        } else {
            self.globals.clone()
        };
        Self {
            chunk: Chunk::new(),
            scopes: vec![Scope {
                bindings: HashMap::new(),
                slots: 0,
            }],
            function: Some(kind),
            globals,
            enclosing,
        }
    }

    /// Whether `scope` is the program's global scope.
    fn is_global_scope(&self, scope: usize) -> bool {
        scope == 0 && self.function.is_none()
    }

    /// Compiles one function or method body into a template.
    fn compile_function(
        &self,
        name: &str,
        kind: CallableKind,
        params: &[String],
        body: &[Statement],
    ) -> Result<FunctionTemplate, CompileError> {
        let too_many = || CompileError {
            message: "function has too many parameters".into(),
            location: None,
        };
        let param_count = u16::try_from(params.len()).map_err(|_| too_many())?;
        let mut compiler = self.child(kind);
        for param in params {
            compiler.add_binding(0, param, VarKind::Var)?;
        }
        compiler.collect_block_bindings(0, body)?;
        for statement in body {
            compiler.collect_vars(statement)?;
        }
        let enter = compiler.chunk.emit(Instruction::EnterScope(0));
        compiler.emit_scope_declarations(0)?;
        for (position, param) in params.iter().enumerate() {
            let argument = u16::try_from(position).map_err(|_| too_many())?;
            compiler.chunk.emit(Instruction::LoadArgument(argument));
            compiler.emit_initialize(param)?;
        }
        compiler.compile_sequence(body, false)?;
        compiler.chunk.code[enter] = Instruction::EnterScope(compiler.scopes[0].slots);
        compiler.constant(Value::Undefined)?;
        compiler.chunk.emit(Instruction::Return);
        Ok(FunctionTemplate {
            name: name.to_owned(),
            kind,
            param_count,
            chunk: Arc::new(compiler.chunk),
        })
    }

    /// Compiles `program`, returning an owned chunk ready for [`crate::vm::Vm`].
    pub fn compile(mut self, program: &Program) -> Result<Chunk, CompileError> {
        self.collect_root_bindings(program)?;
        let root_enter = self.chunk.emit(Instruction::EnterScope(0));
        self.emit_scope_declarations(0)?;
        self.hoist_functions(&program.body)?;
        self.compile_sequence(&program.body, true)?;
        self.chunk.code[root_enter] = Instruction::EnterScope(self.scopes[0].slots);
        self.chunk.emit(Instruction::Return);
        Ok(self.chunk)
    }

    fn collect_root_bindings(&mut self, program: &Program) -> Result<(), CompileError> {
        self.collect_block_bindings(0, &program.body)?;
        for statement in &program.body {
            self.collect_vars(statement)?;
        }
        Ok(())
    }

    /// Creates the top-level function declarations before the body runs, so
    /// they can be called above their declaration, as in JavaScript.
    fn hoist_functions(&mut self, statements: &[Statement]) -> Result<(), CompileError> {
        for statement in statements {
            if let Statement::FunctionDeclaration { name, params, body } = statement {
                let template = self.compile_function(name, CallableKind::Function, params, body)?;
                let index = self.add_function(template)?;
                self.chunk.emit(Instruction::MakeFunction(index));
                self.emit_initialize(name)?;
            }
        }
        Ok(())
    }

    fn add_function(&mut self, template: FunctionTemplate) -> Result<u16, CompileError> {
        self.chunk
            .add_function(template)
            .map_err(|message| CompileError {
                message: message.into(),
                location: None,
            })
    }

    /// Stores the value on top of the stack into the declared binding `name`,
    /// popping it.
    fn emit_initialize(&mut self, name: &str) -> Result<(), CompileError> {
        let name_index = self.name(name)?;
        let binding = self.binding(name).ok_or_else(|| CompileError {
            message: format!("unresolved declaration `{name}`"),
            location: None,
        })?;
        if binding.global {
            self.chunk.emit(Instruction::InitializeGlobal {
                slot: binding.slot,
                name: name_index,
            });
        } else {
            let depth =
                u16::try_from(self.scopes.len() - 1 - binding.scope).map_err(|_| CompileError {
                    message: "scope nesting is too deep".into(),
                    location: None,
                })?;
            self.chunk.emit(Instruction::InitializeLocal {
                depth,
                slot: binding.slot,
                name: name_index,
            });
        }
        Ok(())
    }

    fn collect_vars(&mut self, statement: &Statement) -> Result<(), CompileError> {
        match statement {
            Statement::VariableDeclaration {
                kind: VarKind::Var,
                name,
                ..
            } => {
                self.add_binding(0, name, VarKind::Var)?;
            }
            Statement::Block(statements) => {
                for statement in statements {
                    self.collect_vars(statement)?;
                }
            }
            Statement::If {
                then_branch,
                else_branch,
                ..
            } => {
                self.collect_vars(then_branch)?;
                if let Some(branch) = else_branch {
                    self.collect_vars(branch)?;
                }
            }
            Statement::While { body, .. } => self.collect_vars(body)?,
            Statement::FunctionDeclaration { .. }
            | Statement::ClassDeclaration(_)
            | Statement::Expression(_)
            | Statement::Return(_)
            | Statement::VariableDeclaration { .. } => {}
        }
        Ok(())
    }

    fn add_binding(
        &mut self,
        scope: usize,
        name: &str,
        kind: VarKind,
    ) -> Result<Binding, CompileError> {
        if let Some(existing) = self.scopes[scope].bindings.get(name).copied() {
            if existing.kind == VarKind::Var && kind == VarKind::Var {
                return Ok(existing);
            }
            return Err(CompileError {
                message: format!("redeclaration of `{name}`"),
                location: None,
            });
        }
        let slot = self.scopes[scope].slots;
        self.scopes[scope].slots = slot.checked_add(1).ok_or_else(|| CompileError {
            message: "scope has too many bindings".into(),
            location: None,
        })?;
        let binding = Binding {
            scope,
            slot,
            kind,
            global: self.is_global_scope(scope),
        };
        self.scopes[scope].bindings.insert(name.to_owned(), binding);
        Ok(binding)
    }

    fn emit_scope_declarations(&mut self, scope: usize) -> Result<(), CompileError> {
        let bindings: Vec<_> = self.scopes[scope]
            .bindings
            .iter()
            .map(|(name, binding)| (name.clone(), *binding))
            .collect();
        for (name, binding) in bindings {
            let name = self.name(&name)?;
            if binding.global {
                self.chunk.emit(Instruction::DeclareGlobal {
                    slot: binding.slot,
                    kind: binding.kind,
                    name,
                });
            } else {
                self.chunk.emit(Instruction::DeclareLocal {
                    slot: binding.slot,
                    kind: binding.kind,
                    name,
                });
            }
        }
        Ok(())
    }

    fn enter_block(&mut self) -> Result<usize, CompileError> {
        let scope = self.scopes.len();
        self.scopes.push(Scope {
            bindings: HashMap::new(),
            slots: 0,
        });
        Ok(scope)
    }

    fn collect_block_bindings(
        &mut self,
        scope: usize,
        statements: &[Statement],
    ) -> Result<(), CompileError> {
        for statement in statements {
            match statement {
                Statement::VariableDeclaration { kind, name, .. } if *kind != VarKind::Var => {
                    self.add_binding(scope, name, *kind)?;
                }
                // Top-level functions are hoisted `var`-style bindings.
                Statement::FunctionDeclaration { name, .. } if self.is_global_scope(scope) => {
                    self.add_binding(scope, name, VarKind::Var)?;
                }
                Statement::ClassDeclaration(class) => {
                    self.add_binding(scope, &class.name, VarKind::Let)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn binding(&self, name: &str) -> Option<Binding> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.bindings.get(name).copied())
            .or_else(|| self.globals.get(name).copied())
    }

    /// Resolves a name for reading or assignment. A name that belongs to an
    /// enclosing function or block is an error because bytecode functions do
    /// not capture variables.
    fn resolve(&self, name: &str) -> Result<Option<Binding>, CompileError> {
        let own = self
            .scopes
            .iter()
            .rev()
            .find_map(|scope| scope.bindings.get(name).copied());
        if own.is_none() && self.enclosing.contains(name) {
            return Err(CompileError::unsupported(&format!(
                "functions that capture the enclosing local `{name}`"
            )));
        }
        Ok(own.or_else(|| self.globals.get(name).copied()))
    }

    fn name(&mut self, name: &str) -> Result<u16, CompileError> {
        self.chunk.add_name(name).map_err(|message| CompileError {
            message: message.into(),
            location: None,
        })
    }

    fn compile_sequence(
        &mut self,
        statements: &[Statement],
        wants_completion: bool,
    ) -> Result<bool, CompileError> {
        let mut has_value = false;
        for statement in statements {
            let produced = self.compile_statement(statement, true)?;
            if produced {
                if has_value {
                    self.chunk.emit(Instruction::Swap);
                    self.chunk.emit(Instruction::Pop);
                }
                has_value = true;
            }
        }
        if wants_completion {
            if !has_value {
                self.constant(Value::Undefined)?;
            }
            Ok(true)
        } else {
            if has_value {
                self.chunk.emit(Instruction::Pop);
            }
            Ok(false)
        }
    }

    fn compile_statement(
        &mut self,
        statement: &Statement,
        is_completion: bool,
    ) -> Result<bool, CompileError> {
        match statement {
            Statement::Expression(expression) => {
                self.compile_expression(expression)?;
                if !is_completion {
                    self.chunk.emit(Instruction::Pop);
                }
                Ok(is_completion)
            }
            Statement::Block(statements) => {
                let scope = self.enter_block()?;
                self.collect_block_bindings(scope, statements)?;
                let enter = self.chunk.emit(Instruction::EnterScope(0));
                self.emit_scope_declarations(scope)?;
                self.compile_sequence(statements, true)?;
                self.chunk.code[enter] = Instruction::EnterScope(self.scopes[scope].slots);
                self.chunk.emit(Instruction::ExitScope);
                self.scopes.pop();
                if !is_completion {
                    self.chunk.emit(Instruction::Pop);
                }
                Ok(is_completion)
            }
            Statement::VariableDeclaration { kind, name, init } => {
                if let Some(init) = init {
                    self.compile_expression(init)?;
                } else {
                    self.constant(Value::Undefined)?;
                }
                self.emit_initialize(name)?;
                let _ = kind;
                Ok(false)
            }
            Statement::If {
                condition,
                then_branch,
                else_branch,
            } => self.compile_if(condition, then_branch, else_branch, is_completion),
            Statement::While { condition, body } => {
                self.compile_while(condition, body, is_completion)
            }
            Statement::FunctionDeclaration { .. }
                if self.function.is_none() && self.scopes.len() == 1 =>
            {
                // Created before the program body ran; see `hoist_functions`.
                Ok(false)
            }
            Statement::FunctionDeclaration { .. } => Err(CompileError::unsupported(
                "function declarations outside the top level",
            )),
            Statement::ClassDeclaration(class) => {
                self.compile_class(class)?;
                Ok(false)
            }
            Statement::Return(value) => {
                if self.function.is_none() {
                    return Err(CompileError::unsupported("return outside a function"));
                }
                match value {
                    Some(value) => self.compile_expression(value)?,
                    None => self.constant(Value::Undefined)?,
                }
                self.chunk.emit(Instruction::Return);
                // Unreachable, but keeps the stack shape that callers expect.
                if is_completion {
                    self.constant(Value::Undefined)?;
                }
                Ok(is_completion)
            }
        }
    }

    fn compile_expression(&mut self, expression: &Expr) -> Result<(), CompileError> {
        match expression {
            Expr::Number(value) => self.constant(Value::Number(*value)),
            Expr::String(value) => self.constant(Value::String(value.clone())),
            Expr::Boolean(value) => self.constant(Value::Boolean(*value)),
            Expr::Null => self.constant(Value::Null),
            Expr::Undefined => self.constant(Value::Undefined),
            Expr::Unary { operator, operand } => {
                self.compile_expression(operand)?;
                self.chunk.emit(Instruction::Unary(*operator));
                Ok(())
            }
            Expr::Binary {
                left,
                operator,
                right,
            } => {
                self.compile_expression(left)?;
                self.compile_expression(right)?;
                self.chunk.emit(Instruction::Binary(*operator));
                Ok(())
            }
            Expr::Logical {
                left,
                operator,
                right,
            } => self.compile_logical(left, *operator, right),
            Expr::Identifier(name) => self.compile_load(name),
            Expr::Assign { name, value } => {
                self.compile_expression(value)?;
                self.compile_store(name)
            }
            Expr::This => {
                self.chunk.emit(Instruction::LoadThis);
                Ok(())
            }
            Expr::Call { callee, arguments } => self.compile_call(callee, arguments),
            Expr::New { callee, arguments } => {
                self.compile_expression(callee)?;
                let count = self.compile_arguments(arguments)?;
                self.chunk.emit(Instruction::New(count));
                Ok(())
            }
            Expr::SuperCall { arguments } => {
                if self.function != Some(CallableKind::DerivedConstructor) {
                    return Err(CompileError {
                        message: "`super(...)` is only valid in a derived class constructor".into(),
                        location: None,
                    });
                }
                let count = self.compile_arguments(arguments)?;
                self.chunk.emit(Instruction::SuperCall(count));
                Ok(())
            }
            Expr::SuperMember { name } => {
                self.require_super()?;
                let index = self.name_constant(name)?;
                self.chunk.emit(Instruction::LoadSuperProperty(index));
                Ok(())
            }
            Expr::Member { object, property } => {
                self.compile_expression(object)?;
                match property {
                    MemberProperty::Named(name) => {
                        let index = self.name_constant(name)?;
                        self.chunk.emit(Instruction::LoadProperty(index));
                    }
                    MemberProperty::Computed(key) => {
                        self.compile_expression(key)?;
                        self.chunk.emit(Instruction::LoadComputedProperty);
                    }
                }
                Ok(())
            }
            Expr::MemberAssign {
                object,
                property,
                value,
            } => {
                self.compile_expression(object)?;
                match property {
                    MemberProperty::Named(name) => {
                        let index = self.name_constant(name)?;
                        self.compile_expression(value)?;
                        self.chunk.emit(Instruction::StoreProperty(index));
                    }
                    MemberProperty::Computed(key) => {
                        self.compile_expression(key)?;
                        self.compile_expression(value)?;
                        self.chunk.emit(Instruction::StoreComputedProperty);
                    }
                }
                Ok(())
            }
            Expr::Object(properties) => {
                self.chunk.emit(Instruction::NewObject);
                for property in properties {
                    match &property.key {
                        PropertyKey::Named(name) => {
                            let index = self.name_constant(name)?;
                            self.compile_expression(&property.value)?;
                            self.chunk.emit(Instruction::DefineProperty(index));
                        }
                        PropertyKey::Computed(key) => {
                            self.compile_expression(key)?;
                            self.compile_expression(&property.value)?;
                            self.chunk.emit(Instruction::DefineComputedProperty);
                        }
                    }
                }
                Ok(())
            }
        }
    }

    fn require_super(&self) -> Result<(), CompileError> {
        match self.function {
            Some(kind) if kind != CallableKind::Function => Ok(()),
            _ => Err(CompileError {
                message: "`super` is only valid inside a class method".into(),
                location: None,
            }),
        }
    }

    fn compile_arguments(&mut self, arguments: &[Expr]) -> Result<u16, CompileError> {
        for argument in arguments {
            self.compile_expression(argument)?;
        }
        u16::try_from(arguments.len()).map_err(|_| CompileError {
            message: "call has too many arguments".into(),
            location: None,
        })
    }

    /// Lowers a call. A member callee is a method call: the object is kept
    /// below the function so it becomes `this`.
    fn compile_call(&mut self, callee: &Expr, arguments: &[Expr]) -> Result<(), CompileError> {
        match callee {
            Expr::Member { object, property } => {
                self.compile_expression(object)?;
                self.chunk.emit(Instruction::Dup);
                match property {
                    MemberProperty::Named(name) => {
                        let index = self.name_constant(name)?;
                        self.chunk.emit(Instruction::LoadProperty(index));
                    }
                    MemberProperty::Computed(key) => {
                        self.compile_expression(key)?;
                        self.chunk.emit(Instruction::LoadComputedProperty);
                    }
                }
                let count = self.compile_arguments(arguments)?;
                self.chunk.emit(Instruction::CallMethod(count));
            }
            Expr::SuperMember { name } => {
                self.require_super()?;
                self.chunk.emit(Instruction::LoadThis);
                let index = self.name_constant(name)?;
                self.chunk.emit(Instruction::LoadSuperProperty(index));
                let count = self.compile_arguments(arguments)?;
                self.chunk.emit(Instruction::CallMethod(count));
            }
            _ => {
                self.compile_expression(callee)?;
                let count = self.compile_arguments(arguments)?;
                self.chunk.emit(Instruction::Call(count));
            }
        }
        Ok(())
    }

    /// Lowers a class declaration: the optional superclass value, the
    /// constructor, each method, then the binding the class is stored in.
    fn compile_class(&mut self, class: &ClassDefinition) -> Result<(), CompileError> {
        let has_superclass = class.superclass.is_some();
        if let Some(superclass) = &class.superclass {
            self.compile_expression(superclass)?;
        }
        let (kind, params, body) = match (&class.constructor, has_superclass) {
            (Some(method), true) => (
                CallableKind::DerivedConstructor,
                &method.params[..],
                &method.body[..],
            ),
            (Some(method), false) => (
                CallableKind::BaseConstructor,
                &method.params[..],
                &method.body[..],
            ),
            (None, true) => (CallableKind::DefaultDerivedConstructor, &[][..], &[][..]),
            (None, false) => (CallableKind::BaseConstructor, &[][..], &[][..]),
        };
        let template = self.compile_function(&class.name, kind, params, body)?;
        let function = self.add_function(template)?;
        self.chunk.emit(Instruction::DefineClass {
            function,
            has_superclass,
        });
        for method in &class.methods {
            let template = self.compile_function(
                &method.name,
                CallableKind::Method,
                &method.params,
                &method.body,
            )?;
            let function = self.add_function(template)?;
            let name = self.name_constant(&method.name)?;
            self.chunk
                .emit(Instruction::DefineMethod { function, name });
        }
        self.emit_initialize(&class.name)
    }

    /// Adds a property name to the constant pool and returns its index.
    fn name_constant(&mut self, name: &str) -> Result<u16, CompileError> {
        self.chunk
            .add_constant(Value::String(name.to_owned()))
            .map_err(|message| CompileError {
                message: message.into(),
                location: None,
            })
    }

    fn compile_load(&mut self, name: &str) -> Result<(), CompileError> {
        let name_index = self.name(name)?;
        let Some(binding) = self.resolve(name)? else {
            self.chunk
                .emit(Instruction::LoadUnresolved { name: name_index });
            return Ok(());
        };
        if binding.global {
            self.chunk.emit(Instruction::LoadGlobal {
                slot: binding.slot,
                name: name_index,
            });
        } else {
            let depth =
                u16::try_from(self.scopes.len() - 1 - binding.scope).map_err(|_| CompileError {
                    message: "scope nesting is too deep".into(),
                    location: None,
                })?;
            self.chunk.emit(Instruction::LoadLocal {
                depth,
                slot: binding.slot,
                name: name_index,
            });
        }
        Ok(())
    }

    fn compile_store(&mut self, name: &str) -> Result<(), CompileError> {
        let name_index = self.name(name)?;
        let Some(binding) = self.resolve(name)? else {
            self.chunk
                .emit(Instruction::StoreUnresolved { name: name_index });
            return Ok(());
        };
        if binding.global {
            self.chunk.emit(Instruction::StoreGlobal {
                slot: binding.slot,
                name: name_index,
            });
        } else {
            let depth =
                u16::try_from(self.scopes.len() - 1 - binding.scope).map_err(|_| CompileError {
                    message: "scope nesting is too deep".into(),
                    location: None,
                })?;
            self.chunk.emit(Instruction::StoreLocal {
                depth,
                slot: binding.slot,
                name: name_index,
            });
        }
        Ok(())
    }

    fn compile_logical(
        &mut self,
        left: &Expr,
        operator: LogicalOperator,
        right: &Expr,
    ) -> Result<(), CompileError> {
        self.compile_expression(left)?;
        let jump = match operator {
            LogicalOperator::And => self.chunk.emit(Instruction::JumpIfFalse(usize::MAX)),
            LogicalOperator::Or => self.chunk.emit(Instruction::JumpIfTrue(usize::MAX)),
        };
        self.chunk.emit(Instruction::Pop);
        self.compile_expression(right)?;
        self.chunk.code[jump] = match operator {
            LogicalOperator::And => Instruction::JumpIfFalse(self.chunk.code.len()),
            LogicalOperator::Or => Instruction::JumpIfTrue(self.chunk.code.len()),
        };
        Ok(())
    }

    fn constant(&mut self, value: Value) -> Result<(), CompileError> {
        let index = self
            .chunk
            .add_constant(value)
            .map_err(|message| CompileError {
                message: message.into(),
                location: None,
            })?;
        self.chunk.emit(Instruction::Constant(index));
        Ok(())
    }
}

impl Default for Compiler {
    fn default() -> Self {
        Self::new()
    }
}
