# Writing built-in extensions

Extensions are Rust crates compiled into `scripta` or an embedding host. Register
functions through `scr-runtime`; no interpreter changes or dynamic loader are
needed. Scripts call registered functions directly, without `require`.

This example adds an optional `Greeting::hello` extension. Its crate and feature
are tutorial additions, not shipped APIs. Run commands from the workspace root
with the pinned toolchain.

## Create a crate

Create `crates/scr-ext-greeting/Cargo.toml`:

```toml
[package]
name = "scr-ext-greeting"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
scr-runtime.workspace = true

[lints]
workspace = true
```

The workspace includes `crates/*`. Keep dependencies pointing from the extension
to the runtime, never the reverse. Add backend dependencies to the extension's
manifest. External crates need explicit package metadata and a compatible runtime
dependency instead of workspace inheritance. Preserve the workspace's minimum
Rust version and `unsafe_code = "forbid"` policy.

## Register functions

In `crates/scr-ext-greeting/src/lib.rs`:

```rust
use scr_runtime::{
    NativeError, NativeFunction, NativeResult, NativeSignature, NativeType, Runtime, Value,
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const CAPABILITIES: &[&str] = &["hello"];

pub fn register(runtime: &mut Runtime) -> NativeResult<()> {
    runtime.register(
        "Greeting::hello",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::String], NativeType::String),
            |_context, args| {
                let [Value::String(name)] = args else {
                    return Err(NativeError::type_error("expected one string"));
                };
                if name.trim().is_empty() || name.len() > 256 {
                    return Err(NativeError::new(
                        "GreetingError",
                        "name must contain text and be at most 256 UTF-8 bytes",
                    ));
                }
                Ok(Value::String(format!("Hello, {name}!")))
            },
        ),
    )
}
```

Signatures validate argument counts/types and return types without implicit
conversions. Callbacks validate contents and bound allocations; this example
limits names before formatting. Use a distinct `Namespace::function` name and
propagate registration errors. Duplicate names raise RegistryError; discard a
partially initialized runtime if registration fails.

Return `NativeError` for failures, with useful messages that omit credentials.
The runtime supplies script source locations and traces. Custom error names are
catchable without declaring script classes; `Interrupted` is reserved for
cancellation. Avoid panics and unchecked indexing or unwrap on script input.

Save this as `examples/greeting.scripta`:

```scripta
echo Greeting::hello("Scripta") . "\n";
```

Embedding hosts can call `scr_ext_greeting::register(&mut runtime)?` before
executing source. CLI users need the following wiring.

## Enable the CLI feature

Add to the existing sections in `crates/scr-cli/Cargo.toml`, preserving other
features and defaults:

```toml
[dependencies]
scr-ext-greeting = { path = "../scr-ext-greeting", optional = true }

[features]
greeting = ["dep:scr-ext-greeting"]
```

In `crates/scr-cli/src/extensions.rs`, add to `register()` before its final
`scr_db::register(...)` call:

```rust
#[cfg(feature = "greeting")]
scr_ext_greeting::register(runtime)?;
```

Add the corresponding entry to the vector in `compiled()`:

```rust
#[cfg(feature = "greeting")]
ExtensionInfo {
    name: "greeting",
    version: scr_ext_greeting::VERSION,
    capabilities: scr_ext_greeting::CAPABILITIES,
},
```

This shared registrar covers execution, checking, embedding, script-test workers
and web workers. Register here once, rather than in individual callers. Keep
registration free of I/O: checking registers signatures without executing scripts.
Discovery metadata must describe the API actually registered.

## Handle effects, resources and callbacks

- **Permissions:** use `context.permissions.require("filesystem.read")?` or the
  relevant capability before effects. Capabilities are cooperative checks for
  trusted Rust code, not an OS sandbox. Reuse the standard library's
  [filesystem policy](standard-library.md#filesystem-and-paths) for path/root and
  symlink restrictions; direct Rust filesystem calls do not apply it automatically.
- **Limits:** bound inputs, collections, buffers and outputs before growing them.
  Respect `context.max_value_bytes`, including value/key slots and payloads.
  Logical value limits are not a process heap cap.
- **Cancellation:** check `context.check_cancelled()?` during iterative work and
  propagate Interrupted unchanged. Blocking backend calls need their own deadlines;
  the interpreter cannot interrupt arbitrary Rust work.
- **Handles:** use `Value::NativeObject(Rc<dyn NativeObject>)` and
  `NativeType::Object("YourType")`. Validate concrete types, runtime ownership
  (`Runtime::native_identity()` / `context.runtime_identity`) and closed state.
  Expose operations as `Module::operation(handle, ...)`; native properties are
  supported, but native handles do not have a general instance-method API.
- **Cleanup:** use RAII/Drop and idempotent close operations. Script finally may be
  bypassed by cancellation or fatal limits. Avoid ownership cycles and report
  retained logical data through `NativeObject::payload_bytes`. For resources that
  must expire after each execution, use `Runtime::add_execution_cleanup` with an
  idempotent callback that neither panics nor executes script code; see
  [temporary resources](../crates/scr-stdlib/src/temporary.rs).
- **Values and output:** construct collections with `Value::list(values)` and
  `Value::map(entries)`; preserve unique map keys and valid values. Use
  `Value::Bytes` / `NativeType::Bytes` for binary data. Write through
  `context.output` to share script output limits; flush only when the API promises
  it, since flushing can commit an HTTP response.

For script callbacks, use `NativeFunction::reentrant(signature, callback)`.
Its `InvocationContext` provides `invoke`, `invoke_owned`, `checkpoint`,
`require_permission` and `max_value_bytes()`. Propagate InvocationResult errors
unchanged to preserve exceptions, exit, cancellation and fatal limits. Callback
invocations share the execution budget; they do not reset it.

See [runtime context and errors](standard-library.md#runtime-context-and-errors)
for the full native contract, [SQLite](sqlite.md) for stateful handles, and the
[database connector guide](database-connectors.md) for shared Driver/Connection
implementations. Db drivers supply discovery metadata through `Driver::info`.

## Build and test

Replace `TARGET` with your native triple from `rustc -vV`. On Apple Silicon macOS,
use `aarch64-apple-darwin` and set `MACOSX_DEPLOYMENT_TARGET=15.0`.
The first command intentionally updates Cargo.lock for the new crate:

```sh
cargo check -p scr-cli --features greeting --target TARGET
cargo run -p scr-cli --locked --features greeting --target TARGET -- run examples/greeting.scripta
cargo test -p scr-ext-greeting --locked --target TARGET
cargo test -p scr-cli --features greeting --locked --target TARGET
cargo test -p scr-cli --no-default-features --locked --target TARGET
```

The example prints `Hello, Scripta!`. Test the real CLI path with the feature
both enabled and disabled, including `version --verbose`. Cover valid values,
type/content errors, Unicode, exact limits and limit-plus-one, and duplicate
registration. Stateful APIs also need ownership, denied-permission, failure,
cancellation and cleanup tests. Execute examples; syntax checks alone are not
runtime validation.

Keep Cargo.lock updated and run formatting, all-target/all-feature Clippy and
workspace tests as described in [development](development.md). Validate supported
platforms and dependency/license requirements before distributing binaries.

Users need an extension-enabled executable: `scripta add` cannot compile or load
native Rust extensions. Packages can declare
[native API/capability requirements](packages.md#native-extension-requirements).
Crates may live here or in another repository; pin dependencies in Cargo.lock.
There is no stable shared-library ABI.
