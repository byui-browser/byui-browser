# Contract: JavaScript Engine ↔ Web APIs ↔ DOM

**Teams:** JavaScript Engine (`js`), JS APIs (`webapis`), HTML (`html`, for
the DOM mutation API), DevTools (console consumer).

## Division of labour

| Concern | Crate |
| --- | --- |
| Lexing, parsing, AST, interpreter, `Value`, `Realm`, errors | `js` |
| Generic host-function registration and invocation | `js` |
| Every JavaScript-visible global (`document`, `fetch`, `setTimeout`, `localStorage`, `console`) | `webapis` |
| The event loop / task queue scripts observe | `webapis` |
| The DOM itself and its mutation API | `html` |
| Receiving console output | `devtools` (via a type in `common`) |

The rule that makes this work: **`js` knows nothing about the browser.**
It exposes a way to register a named host function and a way to run source.
Everything with a Web-platform name lives in `webapis`.

## `js` public contract

```rust
pub enum Value { Undefined, Null, Boolean(bool), Number(f64), String(String), Object(ObjectId), Function(FunctionId) }
pub struct JsError { pub message: String /* later: kind, stack */ }
pub type JsResult<T> = Result<T, JsError>;

pub struct Realm { /* global object, interpreter state */ }
impl Realm {
    pub fn new() -> Self;
    pub fn register_global_function(&mut self, name: &str, f: HostFunction) -> JsResult<()>;
    pub fn register_global_value(&mut self, name: &str, value: Value) -> JsResult<()>;
    pub fn evaluate_script(&mut self, source: &str) -> JsResult<Value>;   // full parse + interpret
    pub fn call_function(&mut self, f: FunctionId, this: Value, args: &[Value]) -> JsResult<Value>; // for callbacks
}
pub type HostFunction = Box<dyn FnMut(&mut HostContext<'_>, &[Value]) -> JsResult<Value>>;
pub struct HostContext<'a> { /* access to realm for creating objects/strings; no DOM */ }
```

Non-negotiables:

- `evaluate_script` runs the **real** lexer → parser → interpreter. There is
  no string-matching shortcut. A call expression `print("x")` resolves
  `print` in the global scope, finds a host function, and invokes it through
  the same call path as a script function.
- Host functions are `FnMut`, may capture non-`Send` state, and receive a
  `HostContext` so they can allocate objects/strings without holding a
  second reference to the realm.
- `js` ships no globals of its own except the language's (`Object`, `Math`,
  `JSON` and friends, as they land).
- Execution model is the team's choice (tree-walking interpreter today;
  bytecode later). The public contract does not change when that changes.
- Memory management is the team's choice (`Rc`/arena/GC). `Value` stays
  `Clone`.

## `webapis` public contract

```rust
pub struct HostServices {
    pub document: Rc<RefCell<html::Document>>,
    pub net: Arc<net::RequestController>,
    pub storage: Rc<RefCell<storage::LocalStorage>>,   // per-origin, keyed by security::StorageKey
    pub console: Box<dyn ConsoleSink>,
    pub origin: security::Origin,
}
pub trait ConsoleSink { fn log(&self, message: common::console::ConsoleMessage); }

pub struct ScriptHost { /* realm + event loop + services */ }
impl ScriptHost {
    pub fn new(services: HostServices) -> JsResult<Self>;         // registers every global
    pub fn run_script(&mut self, source: &str) -> JsResult<Value>;
    pub fn run_pending_tasks(&mut self) -> bool;                    // one event-loop turn; true if work remains
    pub fn take_dom_mutations(&mut self) -> Vec<html::Mutation>;   // forwarded for invalidation
}
```

Rules:

- One registration entry point. `browser` calls `ScriptHost::new`; it does
  not call `register_print`, `register_fetch`, ... individually.
- **No blocking.** `fetch` enqueues the request on `net`'s async API and
  returns a pending value; completion is delivered on the next
  `run_pending_tasks`. Spinning up a runtime per call, or `block_on`, is
  forbidden.
- Dotted names (`localStorage.setItem`) are properties on an object, not
  flat globals named with a dot. Until `js` has `Value::Object`, `webapis`
  exposes nothing that needs one and marks the gap.
- `document` is a binding over `html::Document` through its read and
  mutation APIs. `webapis` never copies the DOM into a snapshot struct.
- Timers are real: `setTimeout(cb, ms)` stores `FunctionId` + due time; the
  event loop fires them in order.
- The global named `print` is a scaffold-era placeholder. The standard name
  is `console.log`; `window.print` means "open the print dialog". Rename
  before any page depends on it.

## Console

The console is how every team sees what scripts did, so its message type is
shared:

```rust
// common::console (to be created)
pub enum ConsoleLevel { Log, Info, Warn, Error }
pub struct ConsoleMessage { pub level: ConsoleLevel, pub text: String, pub origin: Option<String>, pub realm: RealmId }
```

`webapis` produces `ConsoleMessage`; `devtools::Console` stores and filters
them; `browser` chooses the sink (stdout in the CLI, DevTools panel in the
UI). `devtools` must not define a parallel `Level`/`LogEntry`.

## Current state (2026-10-03)

- `js::Realm` is a `HashMap<String, HostFunction>` disconnected from the
  interpreter. `evaluate_script` string-matches `name()` and cannot pass
  arguments. The interpreter rejects calls, `if`, `while`, functions and
  `return`. `+` and `==` are numeric only. (G-09)
- `webapis::fetch` creates a tokio runtime and `block_on`s per call. (G-10)
- Registration is split across `register_print`, `register_fetch`,
  `register_local_storage`; `localStorage.setItem` is a flat global with a
  dot in it; `webapis::Document` is a snapshot copy of the DOM with no
  mutation path. (G-11)
- `webapis::ConsoleSink::log(&str)` carries no level and is not connected to
  `devtools::Console`, which has its own `Level`/`LogEntry`. `browser`'s sink
  is `println!`. (G-12)
- `webapis::local_storage::LocalStorage` duplicates `storage::LocalStorage`
  and is a no-op. (G-13)
