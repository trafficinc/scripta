# Writing built-in extensions

A built-in extension is Rust code compiled into the `scripta` executable. It uses
Scripta's existing native registration API to expose functions and host objects.
There is no additional extension framework to implement or runtime loader to
configure. The standard library uses this same API.

The default-enabled [SQLite extension](sqlite.md), implemented in `crates/scr-ext-sqlite`,
provides a complete stateful example with runtime-owned handles and resource limits.

For shared APIs with multiple backend drivers, see the experimental
[database connector foundation](database-connectors.md). Its MySQL crate provides
verified-TLS connections, native prepared statements, typed result fetching,
execution metadata and transactions for embedding hosts and the opt-in CLI mysql
feature. Backends implement the shared TransactionOperation controls with
server-derived state and invalidate handles on uncertain outcomes. Return sanitized
ErrorDetails through Connection::take_error_details for shared post-close
Db::errorInfo snapshots; never supply raw server messages or credentials. The shared
layer owns snapshot limits and reset semantics.

This guide adds a small optional `Greeting::hello(name)` extension in a source
checkout. The crate, feature, and function below are tutorial additions; they
are not shipped or enabled by the current CLI. Keep the pinned Rust toolchain
and run commands from the repository root.

## Create an extension crate

Create this directory structure:

```text
crates/scr-ext-greeting/
    Cargo.toml
    src/lib.rs
examples/greeting.scripta
```

The root workspace already includes `crates/*`, so the new crate becomes a member
without changing the root member list. In `crates/scr-ext-greeting/Cargo.toml`:

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

An extension depends on `scr-runtime`; the runtime must not depend on the
extension. Most extensions need no dependency on `scr-stdlib`. Add any external
Rust libraries to the extension's own manifest. A crate outside this workspace
needs explicit package metadata and a compatible `scr-runtime` dependency in
place of workspace inheritance.

## Register a typed function

In `crates/scr-ext-greeting/src/lib.rs`:

```rust
use scr_runtime::{
    NativeError, NativeFunction, NativeResult, NativeSignature, NativeType, Runtime, Value,
};

pub fn register(runtime: &mut Runtime) -> NativeResult<()> {
    runtime.register(
        "Greeting::hello",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::String], NativeType::String),
            |_context, args| {
                let [Value::String(name)] = args else {
                    return Err(NativeError::type_error(
                        "Greeting::hello() requires one string",
                    ));
                };
                if name.trim().is_empty() || name.len() > 256 {
                    return Err(NativeError::new(
                        "GreetingError",
                        "name must contain non-whitespace text and be at most 256 UTF-8 bytes",
                    ));
                }
                Ok(Value::String(format!("Hello, {name}!")))
            },
        ),
    )
}
```

The signature requires exactly one string and a string return value. Scripta
validates argument count and types before calling Rust, with no implicit
conversions. The callback validates the string's contents and bounds its output
allocation by limiting the input to 256 UTF-8 bytes. It preserves the original
name, including surrounding whitespace. This pure function needs no permission.

Expose a `pub fn register(&mut Runtime) -> NativeResult<()>` by convention; no
trait, global extension registry, or interpreter change is needed. Registration
names can be a bare identifier or one `Namespace::method` pair. Pick a distinct
namespace. Duplicate names raise `RegistryError`; propagate registration errors
rather than silently replacing functions. Multi-function registration is
sequential, so discard a partially initialized runtime if startup fails.

In `examples/greeting.scripta`:

```scr
echo Greeting::hello("Scripta") . "\n";
try {
    Greeting::hello("");
} catch (GreetingError $error) {
    echo "Name rejected\n";
}
```

The successful call returns a value; the script decides when to print it. Named
native errors become catchable script exceptions with source locations. Avoid
panics, indexing into unchecked argument slices, or unwrapping script input.

## Wire the crate into CLI startup

In `crates/scr-cli/Cargo.toml`, add this entry to the existing `[dependencies]`
section:

```toml
scr-ext-greeting = { path = "../scr-ext-greeting", optional = true }
```

Add a `[features]` section, or merge these entries into one if present:

```toml
[features]
default = ["sqlite"]
greeting = ["dep:scr-ext-greeting"]
```

In `crates/scr-cli/src/extensions.rs`, add registration to `register` before its
final `scr_db::register(...)` call:

```rust
    #[cfg(feature = "greeting")]
    scr_ext_greeting::register(runtime)?;
```

This common registrar runs once for every fresh runtime used by the CLI pipeline,
embedding pipeline, static checking, native script-test workers and web workers.
Do not also register the extension in each caller. Checking registers signatures
but performs no script execution or connector I/O; it does not prove driver
availability, a valid DSN or working credentials.

Expose version/capability constants from the extension and add its metadata to the
feature-gated `compiled()` list in this same file so `version --verbose` describes
what is registered. For Db drivers, return version/capabilities from Driver::info;
the CLI reads that same descriptor for both discovery and registration.

The feature is optional and disabled by default. To build this extension into
every default CLI build, add `"greeting"` to the CLI's default feature list.
For an extension that is always mandatory, use a non-optional dependency and
unconditional registration instead.

An embedding host can call `scr_ext_greeting::register(&mut runtime)?` directly
before executing source, without depending on the CLI. Native functions are
available after registration; scripts do not `require` the Rust crate.

## Build and run

First let Cargo update the workspace lockfile for the new crate:

```sh
cargo check -p scr-cli --features greeting
cargo run -p scr-cli --locked --features greeting -- examples/greeting.scripta
```

Expected output:

```text
Hello, Scripta!
Name rejected
```

Build explicitly with the feature disabled:

```sh
cargo build -p scr-cli --locked --no-default-features
cargo run -p scr-cli --locked --no-default-features -- examples/greeting.scripta
```

The build succeeds. The script exits with code 1 and reports
`undefined function Greeting::hello`. `--no-default-features` disables default
features; it does not cancel a feature explicitly enabled through another Cargo
flag or dependency. Use separate Cargo invocations for enabled/disabled checks.

## Test the extension and CLI wiring

Append these tests to `crates/scr-ext-greeting/src/lib.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str) -> Result<String, scr_runtime::RuntimeError> {
        let mut runtime = Runtime::new();
        register(&mut runtime).unwrap();
        let mut output = Vec::new();
        runtime.execute_source(source, &mut output)?;
        Ok(String::from_utf8(output).unwrap())
    }

    #[test]
    fn returns_a_greeting_and_preserves_unicode() {
        assert_eq!(
            run("echo Greeting::hello('世界');").unwrap(),
            "Hello, 世界!"
        );
    }

    #[test]
    fn rejects_invalid_argument_types_and_counts() {
        for call in ["Greeting::hello(42)", "Greeting::hello()", "Greeting::hello('a', 'b')"] {
            let error = run(&format!("{call};")).unwrap_err();
            assert!(error.message.starts_with("TypeError:"), "{error}");
        }
    }

    #[test]
    fn bounds_names_and_exposes_catchable_errors() {
        for name in [" ".to_owned(), "a".repeat(257)] {
            let error = run(&format!("Greeting::hello('{name}');")).unwrap_err();
            assert!(error.message.starts_with("GreetingError:"));
        }
        assert_eq!(
            run("try { Greeting::hello(''); } catch (GreetingError $e) { echo 'caught'; }")
                .unwrap(),
            "caught"
        );
    }

    #[test]
    fn rejects_duplicate_registration() {
        let mut runtime = Runtime::new();
        register(&mut runtime).unwrap();
        assert_eq!(register(&mut runtime).unwrap_err().kind, "RegistryError");
    }
}
```

These unit tests validate the extension API. Also create
`crates/scr-cli/tests/greeting_extension.rs` to verify the actual CLI registration
path with the feature enabled and disabled:

```rust
#[test]
fn greeting_feature_controls_cli_registration() {
    let script = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(script.path(), "echo Greeting::hello('Scripta');").unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_scripta"))
        .arg(script.path())
        .output()
        .unwrap();
    if cfg!(feature = "greeting") {
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, b"Hello, Scripta!");
    } else {
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr)
            .contains("undefined function Greeting::hello"));
    }
}
```

Run both feature configurations independently, then the normal workspace checks:

```sh
cargo test -p scr-ext-greeting --locked
cargo test -p scr-cli --locked --features greeting --test greeting_extension
cargo test -p scr-cli --locked --no-default-features --test greeting_extension
cargo fmt --all
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo test --workspace --no-default-features --locked
```

Keep the new crate and Cargo.lock in version control. In CI, retain checks for
both feature configurations. For larger integrations, add tests for denied
permissions, I/O failures, cancellation, resource closure, and supported platforms.

## Integrations with state or side effects

Use the callback's `RuntimeContext` for host services and metadata. Before an
operation, check the appropriate capability with
`context.permissions.require("filesystem.read")?`, for example. Permission
names are cooperative checks for trusted Rust code; they do not sandbox a crate.
Custom capability names must also be granted by the embedding host. Direct Rust
filesystem calls do not automatically enforce the standard library's filesystem
root policy; an extension must implement any additional host restrictions it
needs. See [runtime context and errors](standard-library.md#runtime-context-and-errors)
and [filesystem policies](standard-library.md#filesystem-and-paths).

Request-owned native resources can compare `context.runtime_identity` with the
opaque `Rc<()>` returned by `Runtime::native_identity()` using `Rc::ptr_eq`.
A missing or different identity must deny access. Also retain a separate host
lifetime guard when a resource must expire before the runtime is dropped, as
multipart upload handles do. The identity carries no backend or permissions.
Embedders constructing `RuntimeContext` directly must supply this field (`None`
for contexts without a runtime owner).

Long-running callbacks should call `context.check_cancelled()?` between bounded
units of work. Blocking library calls also need their own deadlines or timeout
support; the runtime cannot interrupt arbitrary Rust work. Propagate the reserved
`Interrupted` error rather than converting it to an ordinary catchable error.
Respect `context.max_value_bytes` when allocating outputs, and define limits on
input sizes, result counts, and buffers before allocating large amounts of memory.
The greeting example uses a fixed small input bound; it is not a template for
unbounded data processing.

Binary APIs can accept or return `Value::Bytes(Rc<[u8]>)` with
`NativeType::Bytes`. These values have content equality, no implicit text
conversion, and payload-aware runtime value limits. Check output budgets before
allocating byte buffers, as for strings.

For objects retaining in-memory data, implement `NativeObject::payload_bytes`
to report retained logical payload bytes beyond the outer Value slot. The default
is zero; HTTP response objects report raw body and header storage. This hook
should be cheap, side-effect-free, and must not execute script code. It does not
replace allocation checks inside native work or provide an aggregate heap limit.

For stateful handles, return `Value::NativeObject(Rc<dyn NativeObject>)` and use
`NativeType::Object("YourType")` in signatures. Native objects currently expose
properties through `get_property` and optional `set_property`; methods on script
objects are not a general native-method API. A database extension could expose
`Sqlite::open(...)` returning an object, then `Sqlite::query(handle, ...)` and
`Sqlite::close(handle)`. Validate handle ownership and closed state. See
[native objects](standard-library.md#runtime-context-and-errors) and existing
filesystem/process implementations for patterns.

Own resources with Rust RAII, release them on `Drop`, and provide an explicit
idempotent close operation when timely release matters. Avoid native ownership
cycles that keep handles alive. Script `finally` can call close during ordinary
unwinding, but cancellation and fatal execution limits bypass script cleanup.
Rust owners must still release resources when the host drops the runtime and
any other references it holds.

## Distribute an extension-enabled interpreter

Build and package the selected CLI variant:

```sh
cargo build -p scr-cli --release --locked --features greeting
```

Users need that rebuilt `scripta` executable. Installing a script library with
`scripta add` cannot enable a Cargo feature, compile a native extension, or replace
the interpreter. Document which extensions each binary includes; the current CLI
reports built-in extensions in `scripta version --verbose`. Scripts using an unavailable
extension fail when they call its native functions. The shared Db namespace remains
registered without drivers to report DbUnsupportedError and an actionable build
or registration hint.

Built-in extension crates can live in this repository or in separate repositories
referenced through Cargo. Pin external sources in Cargo.lock and build against a
compatible public runtime API. There is no stable shared-library ABI involved.
Check each extension's platform support, native library/linking requirements,
licenses, and release packaging inputs. Static inclusion in `scripta` does not
necessarily mean every external system library is statically linked.

`RuntimeContext.output` is a `ScriptOutput` (`Write` plus generated/remaining byte
accounting). Native extensions use `write_all`, sharing the host writer and
cumulative output limit with `echo`. There is no application buffer control API.
Calling `flush()` flushes the host writer and can commit an HTTP response. Avoid
incidental flushes when an API does not promise them. Hosts continue supplying
an ordinary `Write` to execution and control transport buffering/finalization.
Direct test contexts can use `Vec<u8>`; other direct context writers can implement
`ScriptOutput` with default accounting methods. `RuntimeOptions.max_output_bytes`
limits emitted bytes independently of logical value budgets.

`StdlibOptions.streams` configures live stream handles, chunk sizes, cumulative
transfer bytes, and network timeouts. New stream handles check the runtime's
opaque identity as well as their actual native type before I/O. File streaming
uses the existing filesystem policy; outgoing TCP/UDP use `network.connect` and
`network.datagram`. These are cooperative host capabilities, not an OS sandbox.
Underlying file/socket/HTTP readers and quota leases use RAII; host-retained
handle values retain resources until closed/dropped. A host embedding synchronous
native code must still supply an outer interruption strategy for blocking backend
calls when its own deadline is stricter; the web host terminates worker processes.

When embedding the request-scoped Web module, populate `web::Request.connection`
with trusted `web::ConnectionInfo` metadata, or use `Default::default()` for unknown
connection fields. The standard library exposes this as read-only request
properties and does not parse forwarded headers to infer identity. HTTP hosts
must apply their own explicit proxy trust and allowed-host policy before
registration; `scripta serve` implements that in `scr-web`.

Request hosts may also set `web::Request.request_id: Some(id)` to expose a
read-only `WebRequest.requestId`; use `None` if unknown. Its bytes contribute to
the request payload budget. `scripta serve` generates bounded IDs itself and
correlates worker diagnostics in the supervisor; it never trusts an incoming
request-ID header. Custom hosts own their ID and diagnostic privacy policies.

The standalone host's opt-in development mode uses the existing
`Runtime::set_module_policy` interface to confine includes to its per-generation
source snapshot. Ordinary embeddings and production serving retain their existing
module resolver/policy behavior. Construct host settings through
`scr_web::Config::new`, then set `watch`/`watch_debounce` when needed; generation
snapshot ownership is internal to the host. See
[development watch mode](web-servers.md#development-watch-mode) for supported
include paths and limits.

## Calling script callbacks from a native function

Use `NativeFunction::reentrant(signature, callback)` when a native implementation
must call a script callable. The callback receives `InvocationContext` and its
validated arguments. Call `context.invoke(value, arguments)` and return its
`InvocationResult` to propagate the original exception, exit, or fatal signal.
Use `context.error(NativeError::new(kind, message))` for a new catchable exception
and `context.require_permission(capability)` before effects. `context.checkpoint()`
checks cancellation and instruction limits during iterative work. Reentrant
functions share the output writer, lexical ownership, call depth, and the execution
budget; callback invocation does not reset limits. Functions in the test runner
use this public interface, rather than adding test names to the evaluator.

Hosts may look up a global function with `Runtime::get_callable` and call it with
`Runtime::invoke`. Inspect `InvocationError::exception_kind()` only for catchable
exceptions; propagate errors with no kind so interruption, fatal limits, and exits
are not accidentally swallowed. `diagnostic()`, `exit_code()`, and
`is_interrupted()` expose the corresponding outcomes. Callables from a different
runtime are rejected. Existing `NativeFunction::new` implementations are unchanged.

Script class instances use an internal NativeObject implementation and retain
reference identity when passed to native functions. Native resource APIs must
continue validating concrete handle types and runtime ownership, rather than
trusting a matching `type_name()` string. Script fields and methods are resolved
by their owning runtime; host code should not treat arbitrary NativeObject values
as a native backend handle. See [objects and classes](objects.md).

Native error registration remains unchanged: return `NativeError::new(kind,
message)`, or propagate an `InvocationError` from a reentrant callback. Script
catches receive structured Error objects, including the redacted message,
diagnostic code, source location, and trace. Extension-defined names remain
catchable without declaring a script class. `Interrupted` remains reserved for
cancellation and never becomes catchable. See
[exceptions](language-spec.md#exceptions-and-control-flow).
