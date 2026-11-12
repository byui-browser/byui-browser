//! Owned abstract syntax tree produced by the parser and run by the runtime.

/// An owned JavaScript program in source order.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Program {
    /// Owned statements executed in source order.
    pub body: Vec<Statement>,
}

/// An owned expression tree; grouping and precedence are encoded by nesting.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Numeric literal stored as a 64-bit floating-point value.
    Number(f64),
    /// Owned decoded string literal.
    String(String),
    /// Boolean literal.
    Boolean(bool),
    /// The null literal.
    Null,
    /// The undefined literal.
    Undefined,
    /// Owned name to resolve in the execution environment.
    Identifier(String),
    /// Arrow function with positional parameters and an expression or block body.
    ///
    /// The bytecode VM supports expression and block bodies. Arrow functions
    /// currently share the engine's no-closure limitation and do not capture
    /// enclosing locals or lexical `this`.
    ArrowFunction {
        /// Owned parameter names in argument order.
        params: Vec<String>,
        /// Function body; expression bodies return their value implicitly.
        body: ArrowBody,
    },
    /// Unary operation on one operand.
    Unary {
        /// Operation to apply to the operand.
        operator: UnaryOperator,
        /// Owned operand expression.
        operand: Box<Expr>,
    },
    /// Eager binary operation with ordered operands.
    Binary {
        /// Owned left operand, evaluated first.
        left: Box<Expr>,
        /// Arithmetic or comparison operation.
        operator: BinaryOperator,
        /// Owned right operand.
        right: Box<Expr>,
    },
    /// Logical operation whose right operand may be skipped by short-circuit evaluation.
    Logical {
        /// Owned left operand, evaluated first.
        left: Box<Expr>,
        /// Short-circuit operation controlling right-operand evaluation.
        operator: LogicalOperator,
        /// Owned right operand.
        right: Box<Expr>,
    },
    /// Assignment to an identifier, yielding the assigned value.
    Assign {
        /// Owned identifier spelling.
        name: String,
        /// Owned expression supplying the assigned value.
        value: Box<Expr>,
    },
    /// Function invocation; the callee may itself be a call.
    ///
    /// A member callee such as `object.method()` is read like any other
    /// member and then called; the method does not receive `this`.
    Call {
        /// Owned expression resolving the callable value.
        callee: Box<Expr>,
        /// Owned arguments in evaluation order.
        arguments: Vec<Expr>,
    },
    /// Property read such as `object.name` or `object[key]`.
    ///
    /// The object is evaluated first, then a computed key. A missing property
    /// reads as `undefined`; reading any property of a non-object value is a
    /// runtime error.
    Member {
        /// Owned expression producing the object to read from.
        object: Box<Expr>,
        /// Property name, fixed or computed.
        property: MemberProperty,
    },
    /// The `this` keyword: the receiver of the running method or constructor.
    This,
    /// Construction such as `new Callee(arguments)`.
    ///
    /// The callee is evaluated first, then the arguments in order.
    New {
        /// Owned expression producing the constructor.
        callee: Box<Expr>,
        /// Owned arguments in evaluation order; empty for `new Callee`.
        arguments: Vec<Expr>,
    },
    /// `super(arguments)` inside a derived class constructor: runs the parent
    /// constructor and binds `this` to the instance it produces.
    SuperCall {
        /// Owned arguments in evaluation order.
        arguments: Vec<Expr>,
    },
    /// `super.name` inside a class method: reads `name` starting from the
    /// parent class's prototype. Calling it passes the current `this`.
    SuperMember {
        /// Owned property name.
        name: String,
    },
    /// Property assignment such as `object.name = value`, yielding the
    /// assigned value.
    ///
    /// Evaluation order is the object, then a computed key, then the value;
    /// only then is the object checked, so assigning to a property of a
    /// non-object fails after the value has been evaluated.
    MemberAssign {
        /// Owned expression producing the object to write to.
        object: Box<Expr>,
        /// Property name, fixed or computed.
        property: MemberProperty,
        /// Owned expression supplying the assigned value.
        value: Box<Expr>,
    },
    /// Object literal such as `{ a: 1, 'b': 2, [key]: 3 }`, creating a new
    /// object each time it is evaluated.
    ///
    /// Properties are evaluated in source order, each key before its value.
    /// A repeated key keeps its first position and its last value. Shorthand
    /// `{ a }` is parsed as `{ a: a }`. Spread (`...source`) is supported.
    /// Methods, getters, and setters are unsupported.
    Object(Vec<ObjectProperty>),
    /// Array literal. `None` entries represent elisions and evaluate to
    /// `undefined` when read by index. An entry may be an [`Expr::Spread`],
    /// which contributes each element of an array or each character of a
    /// string.
    Array(Vec<Option<Expr>>),
    /// `test ? consequent : alternate`. Only the selected branch is evaluated.
    Conditional {
        /// Owned condition, evaluated first and tested for truthiness.
        test: Box<Expr>,
        /// Evaluated only when the condition is truthy.
        consequent: Box<Expr>,
        /// Evaluated only when the condition is falsy.
        alternate: Box<Expr>,
    },
    /// Comma expression `a, b, c`: evaluates every operand left to right and
    /// yields the last one. Always has at least two operands.
    Sequence(Vec<Expr>),
    /// `++x`, `x++`, `--x`, or `x--` on an identifier or member expression.
    ///
    /// The old value is converted with `ToNumber` first. A prefix update
    /// yields the new value and a postfix update yields the converted old
    /// value.
    Update {
        /// Identifier or [`Expr::Member`] being updated; the parser rejects
        /// every other target.
        target: Box<Expr>,
        /// Whether the value is incremented or decremented.
        operator: UpdateOperator,
        /// `true` for `++x`/`--x`, `false` for `x++`/`x--`.
        prefix: bool,
    },
    /// Compound assignment such as `x += 1` or `object.count ||= 0`.
    ///
    /// The target's object and key are evaluated once. Logical forms
    /// (`&&=`, `||=`, `??=`) skip both the right operand and the store when
    /// the left value already decides the result.
    CompoundAssign {
        /// Identifier or [`Expr::Member`] being assigned; the parser rejects
        /// every other target.
        target: Box<Expr>,
        /// Operation combining the old value with the right operand.
        operator: AssignmentOperator,
        /// Owned right operand.
        value: Box<Expr>,
    },
    /// `delete object.name` or `delete object[key]`, yielding a boolean.
    ///
    /// Deleting a property of `null` or `undefined` is a runtime error. An
    /// operand of any other shape is evaluated and the result is `true`.
    /// `delete name` on a bare identifier and `delete` on an optional chain
    /// are rejected while parsing.
    Delete(Box<Expr>),
    /// Template literal such as `` `a${b}c` ``.
    ///
    /// `quasis` always has exactly one more entry than `expressions`, and the
    /// parts are joined in order. Tagged templates are unsupported.
    Template {
        /// Decoded literal text between substitutions.
        quasis: Vec<String>,
        /// Substitution expressions in source order.
        expressions: Vec<Expr>,
    },
    /// `...value` inside an array literal or call or `new` arguments. (Object
    /// literals use [`PropertyKey::Spread`].) Never appears elsewhere.
    Spread(Box<Expr>),
    /// The extent of an optional chain such as `a?.b.c(d)`.
    ///
    /// If any [`Expr::OptionalMember`] or [`Expr::OptionalCall`] inside finds
    /// a `null` or `undefined` base, the rest of the chain is skipped and the
    /// whole chain evaluates to `undefined`.
    OptionalChain(Box<Expr>),
    /// `object?.name` or `object?.[key]` inside an [`Expr::OptionalChain`].
    OptionalMember {
        /// Base expression; `null` or `undefined` short-circuits the chain.
        object: Box<Expr>,
        /// Property name, fixed or computed.
        property: MemberProperty,
    },
    /// `callee?.(arguments)` inside an [`Expr::OptionalChain`]. A `null` or
    /// `undefined` callee short-circuits the chain before any argument runs.
    OptionalCall {
        /// Expression producing the callee.
        callee: Box<Expr>,
        /// Owned arguments in evaluation order.
        arguments: Vec<Expr>,
    },
}

/// `++` or `--` in an [`Expr::Update`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateOperator {
    /// `++`: adds one.
    Increment,
    /// `--`: subtracts one.
    Decrement,
}

/// The operator of an [`Expr::CompoundAssign`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignmentOperator {
    /// An eager binary operator: `+=`, `-=`, `*=`, `/=`, `%=`, `**=`, `<<=`,
    /// `>>=`, `>>>=`, `&=`, `|=`, or `^=`.
    Binary(BinaryOperator),
    /// A short-circuiting operator: `&&=`, `||=`, or `??=`.
    Logical(LogicalOperator),
}

/// Body of an arrow function.
#[derive(Debug, Clone, PartialEq)]
pub enum ArrowBody {
    /// Expression returned implicitly when the function is called.
    Expression(Box<Expr>),
    /// Statements executed in source order; an explicit return supplies the result.
    Block(Vec<Statement>),
}

/// One `key: value` entry of an object literal.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectProperty {
    /// Property name, fixed or computed.
    pub key: PropertyKey,
    /// Owned expression supplying the property's value.
    pub value: Expr,
}

/// The key part of an object literal property.
#[derive(Debug, Clone, PartialEq)]
pub enum PropertyKey {
    /// A fixed name written as an identifier (keywords allowed), a string,
    /// or a number. Numbers are stored in their JavaScript string form, so
    /// `{ 1.50: x }` has the key `"1.5"`.
    Named(String),
    /// `[expression]`: the key is converted to a string after evaluation.
    Computed(Box<Expr>),
    /// `...value`: the entry has no key of its own; its value expression
    /// supplies the source whose own enumerable properties are copied. `null`
    /// and `undefined` sources are ignored, arrays and strings contribute
    /// their indices, and other values contribute nothing.
    Spread,
}

/// The property part of a member expression.
#[derive(Debug, Clone, PartialEq)]
pub enum MemberProperty {
    /// `object.name`: an owned property name, which may be a keyword.
    Named(String),
    /// `object[expression]`: the key is converted to a string after
    /// evaluation, so `object[1]` and `object['1']` name the same property.
    Computed(Box<Expr>),
}

/// An owned statement in the supported program grammar.
#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    /// Owned statements enclosed in braces, in source order.
    Block(Vec<Statement>),
    /// Expression executed for its effects and completion value.
    Expression(Expr),
    /// Declaration of one named variable.
    VariableDeclaration {
        /// Declaration keyword and its binding semantics.
        kind: VarKind,
        /// Owned identifier spelling.
        name: String,
        /// Optional initializer; constants require one when parsed.
        init: Option<Expr>,
    },
    /// Conditional statement with an optional alternative.
    If {
        /// Owned expression tested for truthiness.
        condition: Expr,
        /// Statement executed when the condition is truthy.
        then_branch: Box<Statement>,
        /// Optional statement executed when the condition is falsy.
        else_branch: Option<Box<Statement>>,
    },
    /// Loop that tests its condition before each iteration.
    While {
        /// Owned expression tested for truthiness.
        condition: Expr,
        /// Owned statement executed on each iteration.
        body: Box<Statement>,
    },
    /// Loop with optional initialization, condition, and update clauses.
    For {
        /// Optional expression or variable declaration evaluated once before
        /// the first condition check.
        init: Option<Box<Statement>>,
        /// Optional expression tested before each iteration; absent means true.
        condition: Option<Expr>,
        /// Optional expression evaluated after each body iteration.
        update: Option<Expr>,
        /// Owned statement executed on each iteration.
        body: Box<Statement>,
    },
    /// Named function declaration with an owned body.
    FunctionDeclaration {
        /// Owned identifier spelling.
        name: String,
        /// Owned parameter names in argument order.
        params: Vec<String>,
        /// Owned function statements in source order.
        body: Vec<Statement>,
    },
    /// Function return with an optional value; absence means undefined.
    Return(Option<Expr>),
    /// Class declaration with an optional superclass.
    ClassDeclaration(Box<ClassDefinition>),
    /// `throw expression;`: raises any value as an exception.
    Throw(Expr),
    /// `try { ... } catch (name) { ... } finally { ... }`.
    ///
    /// At least one of `handler` and `finalizer` is present. The completion
    /// value is that of the `try` block, or of the `catch` block when it
    /// ran; a `finally` block changes it only by completing abruptly.
    Try {
        /// Statements of the protected block.
        block: Vec<Statement>,
        /// The `catch` clause, if any.
        handler: Option<CatchClause>,
        /// Statements of the `finally` block, if any. They run on every
        /// exit of the `try` and `catch` blocks, including `return`, `break`,
        /// `continue`, and a thrown exception.
        finalizer: Option<Vec<Statement>>,
    },
    /// `break;`: leaves the innermost enclosing loop.
    Break,
    /// `continue;`: starts the next iteration of the innermost enclosing
    /// loop. Labels are unsupported.
    Continue,
}

/// The `catch` clause of a [`Statement::Try`].
#[derive(Debug, Clone, PartialEq)]
pub struct CatchClause {
    /// Name bound to the caught value, or `None` for `catch { ... }`.
    /// Destructuring parameters are unsupported.
    pub param: Option<String>,
    /// Statements of the clause body, which has its own block scope.
    pub body: Vec<Statement>,
}

/// The parsed body of a class declaration.
///
/// Limitations: only plain methods and one constructor are represented.
/// Static members, accessors, fields, computed member names, and class
/// expressions are not supported.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassDefinition {
    /// Owned class name, which also names the binding the class is stored in.
    pub name: String,
    /// Expression after `extends`, evaluated once when the class is defined.
    pub superclass: Option<Expr>,
    /// The explicit `constructor`, or `None` for the implicit one.
    pub constructor: Option<MethodDefinition>,
    /// Prototype methods in source order.
    pub methods: Vec<MethodDefinition>,
}

/// A method or constructor inside a class body.
#[derive(Debug, Clone, PartialEq)]
pub struct MethodDefinition {
    /// Owned method name.
    pub name: String,
    /// Owned parameter names in argument order.
    pub params: Vec<String>,
    /// Owned body statements in source order.
    pub body: Vec<Statement>,
}

/// Keyword used to declare a binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarKind {
    /// Function-scoped variable declaration (`var`).
    Var,
    /// Block-scoped mutable declaration (`let`).
    Let,
    /// Block-scoped immutable declaration (`const`).
    Const,
}

/// Supported single-operand operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOperator {
    /// `-x`
    Negate,
    /// `!x`
    Not,
    /// `~x`
    BitwiseNot,
    /// `+x`: converts the operand with `ToNumber`.
    Plus,
    /// `typeof x`: the operand's type name as a string. A bare identifier
    /// that is not declared yields `"undefined"` instead of an error.
    Typeof,
    /// `void x`: evaluates the operand and yields `undefined`.
    Void,
}

/// Arithmetic, bitwise, and comparison operations represented by the AST.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOperator {
    /// `+` operator.
    Add,
    /// `-` operator.
    Subtract,
    /// `*` operator.
    Multiply,
    /// `/` operator.
    Divide,
    /// `%` operator; the result takes the sign of the dividend.
    Remainder,
    /// `&` operator on 32-bit integers.
    BitwiseAnd,
    /// `|` operator on 32-bit integers.
    BitwiseOr,
    /// `^` operator on 32-bit integers.
    BitwiseXor,
    /// `<<` operator; the shift count is taken modulo 32.
    LeftShift,
    /// `>>` sign-propagating operator; the shift count is taken modulo 32.
    SignedRightShift,
    /// `>>>` zero-fill operator; the shift count is taken modulo 32.
    UnsignedRightShift,
    /// `<` operator.
    Less,
    /// `<=` operator.
    LessEqual,
    /// `>` operator.
    Greater,
    /// `>=` operator.
    GreaterEqual,
    /// `==` operator.
    Equal,
    /// `!=` operator.
    NotEqual,
    /// `===` operator.
    StrictEqual,
    /// `!==` operator.
    StrictNotEqual,
    /// `**` operator; right associative.
    Exponent,
    /// `key in object`: whether the property exists on the object or its
    /// prototype chain. The right operand must be an object, array, or
    /// function.
    In,
    /// `value instanceof Constructor`: whether the constructor's `prototype`
    /// is on the value's prototype chain.
    InstanceOf,
}

/// Operations represented separately to preserve short-circuit structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalOperator {
    /// `&&`: evaluate the right operand only if the left is truthy.
    And,
    /// `||`: evaluate the right operand only if the left is falsy.
    Or,
    /// `??`: evaluate the right operand only if the left is `null` or
    /// `undefined`.
    Nullish,
}
