# Architecture

Scripta is a synchronous AST interpreter written in safe Rust:
**source → tokens → AST → evaluation**. Parsing finishes before execution.

## Crate boundaries

| Crate | Responsibility |
| --- | --- |
| `scr-diagnostics` | Source spans and diagnostic rendering |
| `scr-ast` | Shared syntax structures |
| `scr-lexer` | Tokenization and interpolation tokens |
| `scr-parser` | Parsing, syntax checks and recovery |
| `scr-runtime` | Values, scopes, evaluation, objects and native interfaces |
| `scr-stdlib` | Automation APIs and host policies |
| `scr-ext-sqlite` | SQLite backend and database handles |
| `scr-web` | HTTP supervision and interpreter workers |
| `scr-cli` | Commands, formatting, packages and worker entry points |
| `scr-release` | Distribution packaging and verification |

The runtime depends on syntax crates, not the standard library or its backends.
Extensions register through public native interfaces. The `scripta` and legacy
`scr` commands share one CLI entry point; SQLite is enabled by default.

## Parsing and diagnostics

The parser uses recursive descent for statements and Pratt parsing for
expressions. Recovery reports errors without returning a partial executable
program. Source spans use UTF-8 byte offsets; diagnostics render Unicode scalar
columns and retain defining sources and call sites across loads and callbacks.

## Runtime ownership and scopes

`Runtime` owns an arena of lexical scopes. Functions retain declaration metadata,
lexical scope and runtime identity. Collections have value semantics; objects
and callables have identity semantics. See the [language specification](language-spec.md).

Uncaptured scopes are released on exit. Tracing reclaims unreachable captured
scopes and script-object cycles, preserving active calls, cached modules and
externally held values. Opaque native ownership cycles require host cleanup.
[Classes and interfaces](objects.md) use runtime-owned type identities and fields.

## Execution hot paths and limits

Bounded numeric plans, compact instruction plans for eligible scalar function
bodies, checked lookup hints and storage reuse accelerate common cases. Ordinary
evaluation handles unsupported cases.

`RuntimeOptions` controls execution and logical resource budgets; these are not
an aggregate heap cap. Internal signals preserve exceptions, returns and exits.
Cancellation and fatal limits bypass script cleanup; Rust ownership handles
resource release. See [resource behavior](standard-library.md#distribution-and-resource-behavior).

## Native APIs and I/O

Typed native registration validates arguments and results. Runtime contexts
supply streams, permissions, cancellation and host state. Reentrant invocation
shares script scopes, budgets and error signals without terminating the host.

OS, network and database backends belong in standard-library or extension crates.
They enforce capabilities, resource ownership and operation-specific limits.
Output buffering is shared across script and native calls. See
[writing extensions](writing-extensions.md) and the [standard library](standard-library.md).

## Source loading and developer tools

The loader checks file identity and host policy. Ordinary loads use the current
scope; aliased modules use isolated, traced environments and cache successful
initialization. See [script loading](language-spec.md#local-script-loading).

Execution, checking and formatting share the CLI parsing pipeline. Checking does
not execute code; formatting validates output before atomic publication. Packages
and managed test processes belong to the CLI. See [development](development.md),
[packages](packages.md) and [testing](testing.md).

## Web request hosting

`scripta serve` uses an asynchronous supervisor and a bounded interpreter worker
pool. Workers cache parsed entry scripts and create a fresh runtime per request.
The supervisor owns admission, deadlines, worker lifecycle and request files;
web APIs register through the standard library. See [web hosting](web-servers.md).
