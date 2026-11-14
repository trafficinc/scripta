# Writing built-in extensions

A built-in extension is Rust code compiled into the `scripta` executable. It uses
Scripta's existing native registration API to expose functions and host objects.
There is no additional extension framework to implement or runtime loader to
configure. The standard library uses this same API.

The default-enabled [SQLite extension](sqlite.md), implemented in `crates/scr-ext-sqlite`,
provides a complete stateful example with runtime-owned handles and resource limits.

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
<?scripta
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

In `crates/scr-cli/src/pipeline.rs`, inside `run_with_options`, insert the following
immediately after `scr_stdlib::register(...)` succeeds and before
`runtime.execute_source(source, output)`:

```rust
    #[cfg(feature = "greeting")]
    scr_ext_greeting::register(&mut runtime)
        .map_err(|error| Diagnostic::new(error.to_string(), Default::default()))?;
```

`Runtime::with_options` creates a fresh runtime for each pipeline invocation;
register the extension once for each new runtime. This location covers execution
through both the CLI and `scr_cli::pipeline`. Syntax-only `scripta check` does not
execute startup registration or resolve whether a native function exists.

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
            run("<?scripta echo Greeting::hello('世界');").unwrap(),
            "Hello, 世界!"
        );
    }

    #[test]
    fn rejects_invalid_argument_types_and_counts() {
        for call in ["Greeting::hello(42)", "Greeting::hello()", "Greeting::hello('a', 'b')"] {
            let error = run(&format!("<?scripta {call};")).unwrap_err();
            assert!(error.message.starts_with("TypeError:"), "{error}");
        }
    }

    #[test]
    fn bounds_names_and_exposes_catchable_errors() {
        for name in [" ".to_owned(), "a".repeat(257)] {
            let error = run(&format!("<?scripta Greeting::hello('{name}');")).unwrap_err();
            assert!(error.message.starts_with("GreetingError:"));
        }
        assert_eq!(
            run("<?scripta try { Greeting::hello(''); } catch (GreetingError $e) { echo 'caught'; }")
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
    std::fs::write(script.path(), "<?scripta echo Greeting::hello('Scripta');").unwrap();
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
extension fail when they call its native functions.

Built-in extension crates can live in this repository or in separate repositories
referenced through Cargo. Pin external sources in Cargo.lock and build against a
compatible public runtime API. There is no stable shared-library ABI involved.
Check each extension's platform support, native library/linking requirements,
licenses, and release packaging inputs. Static inclusion in `scripta` does not
necessarily mean every external system library is statically linked.
