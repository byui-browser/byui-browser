# `print` JavaScript binding analysis

## Conclusion

The current PR does not create a JavaScript binding yet. It adds a public Rust
function in `crates/webapis/src/lib.rs`, and its unit test calls that Rust
function directly. Nothing currently parses or executes JavaScript, creates a
global object, registers host functions, starts a renderer, or routes output to
the DevTools console. Consequently, JavaScript cannot call `print()` on this
branch.

The PR should be treated as the first slice of a cross-crate integration, not
as a complete Web API implementation. The smallest working path is:

```text
JavaScript source: print()
        -> JS parser/compiler/VM resolves a global host function
        -> JS host-call adapter converts arguments and return value
        -> webapis::print implementation
        -> renderer console sink / DevTools message
```

## What the PR currently provides

`webapis::print()` has this behavior:

```rust
pub fn print() {
    println!("JS API Team Rocks!");
}
```

That proves only that Rust can call the function and that the process stdout
is writable. The existing test named `print_binding_is_callable` is therefore
not a binding test; it never exercises JavaScript, name lookup, argument
conversion, or a browser/renderer integration.

The function also has no dependency on the `js` crate, no registration API,
and no connection to `devtools`. The browser binary is still a scaffold and
does not create a renderer or execute script. The `js` and `devtools` crates
contain TODOs only.

## Changes required

### 1. Define the JavaScript host-function contract in `js`

The JavaScript Engine team needs to add the minimum runtime pieces needed by a
binding. Exact names can follow the engine design, but the contract needs
equivalents of:

- a `Value` type, including at least `Undefined` and `String`;
- a call argument collection, such as `&[Value]`;
- a host-call result/error type;
- a safe host-function callback type;
- a global/realm environment that can register a function by name and resolve
  it during a call; and
- VM support for invoking a host function through the same call path as a
  script function.

A useful shape is conceptually:

```rust
type HostFunction = fn(&[Value], &mut HostContext) -> JsResult<Value>;

realm.register_global_function("print", host_function);
```

The callback should return `undefined` for this API. It should not write to
stdout directly and should report an arity/type error only if the chosen API
contract requires one. The callback type must remain safe Rust and must not
expose VM internals to `webapis`.

### 2. Make `webapis` implement the adapter, not just a Rust helper

`webapis` should expose a binding-registration function, for example
`register_print(realm, console)` or `register_globals(...)`. That function
should register the JavaScript-facing callback under the exact global name
`print` and adapt JS arguments/results to the Rust implementation.

The implementation needs an explicit API decision:

- If this prototype intentionally prints the fixed text `JS API Team Rocks!`,
  document and test `print()` as a zero-argument function.
- If `print(value)` is intended to print caller-provided text, accept the
  argument as a JS value, apply the engine's string conversion rules, and
  test strings, omitted arguments, and non-string values.

The current zero-argument Rust signature cannot represent the common binding
shape `print(...args)`, and the current hard-coded message makes it impossible
for JavaScript to control what is printed. The PR should not silently claim
one behavior while implementing the other.

Because the architecture says that the JS engine owns VM values and Web APIs
own bindings, the likely dependency direction is `webapis -> js`, while `js`
provides only generic registration/call primitives. `js` should not depend on
`webapis`; that would create a crate cycle and put a particular Web API into
the VM.

### 3. Replace `println!` with an owned console/output boundary

`println!` is process stdout, not the browser console. The binding needs an
injected output sink or a renderer console service. For example, `webapis`
could depend on a small trait/context that accepts a console message, while
the renderer supplies the implementation.

The sink should define at least:

- message text and severity (`log`/`info`, or the project's chosen level);
- document/tab or renderer identity, if messages can come from multiple tabs;
- ordering and whether delivery is synchronous or queued; and
- behavior when the DevTools client is not connected.

The renderer can then forward the message to `devtools` through the typed IPC
boundary described in `docs/TECH_ARCHITECTURE.md`. `webapis` should not import
the DevTools UI or assume that stdout is available. If a direct in-process
console sink is chosen for the first vertical slice, the boundary should
still be an interface so it can later be replaced by the renderer/DevTools
transport.

### 4. Register the binding when a JavaScript realm is created

There must be an explicit integration point in the renderer/document startup
path. In order, it should:

1. create the JS VM/realm and its global object;
2. construct the renderer's console sink/context;
3. call the Web API registration function; and
4. execute page script in that same realm.

Registration must happen before evaluating a script containing `print()`.
The top-level `browser` binary currently only emits a scaffolding message, so
the browser/renderer startup path must be added or a focused harness must be
provided before an end-to-end test can pass.

### 5. Add the required crate dependencies and ownership review

At minimum, the implementation will need a deliberate dependency/API update:

- `js` adds the generic VM/value/host-function API;
- `webapis` depends on that public `js` API and registers `print`;
- the renderer or browser composition layer wires `webapis` into a realm;
- `devtools` or `common` defines the console message/transport contract, with
  `common` changes reviewed by all affected teams; and
- the workspace manifests reflect the chosen direction without introducing a
  `js <-> webapis` cycle.

The existing `webapis` dependency on `common` is currently unused by the
binding and does not provide any of these contracts.

### 6. Replace the direct Rust test with layered tests

The current unit test should remain only if it tests a deliberately public
Rust helper. It is not sufficient as the feature test. Add:

1. a `js` unit test for registering and invoking a host function;
2. a `webapis` unit test using a fake console sink, asserting the name,
   arguments, output, and return value;
3. a realm/global-object test asserting that `print` is present before script
   evaluation; and
4. an integration test that evaluates JavaScript source such as `print()` and
   asserts the captured console message.

If the API accepts arguments, include tests for the intended conversion and
error behavior. Also test that a missing DevTools consumer does not panic and
that output from two realms cannot be accidentally mixed.

## API and naming caveat

In browser terminology, `window.print()` conventionally opens the page print
dialog. This PR describes a console-printing function instead. For a prototype
the requested global `print` name can be used, but the eventual Web API should
either implement the standard `window.print` semantics or choose a distinct
name such as `consolePrint`/`hostPrint`. The decision should be documented
before the binding becomes public, because changing the global name later is a
compatibility break.

## Recommended implementation order

1. Agree on the JavaScript signature and the standard-vs-prototype naming
   decision.
2. Have the JS Engine team expose a minimal safe host-function and global
   registration API.
3. Have the Web APIs team implement `print` against an injected console sink.
4. Add realm initialization/registration in the renderer composition layer.
5. Add the fake-sink unit tests and one end-to-end script test.
6. Add the renderer-to-DevTools transport, or explicitly mark the local sink
   as a temporary first-slice implementation.
7. Run `cargo fmt --all`, `cargo test --workspace`, and
   `cargo clippy --workspace --all-targets -- -D warnings`.

## Current verification result

On this branch, `cargo test --workspace` and the configured clippy command pass.
The passing test only calls `webapis::print()` from Rust, so it verifies that
the current crate compiles and writes the hard-coded line to stdout. It does
not verify the requested JavaScript-to-Rust behavior.
