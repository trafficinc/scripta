# Architecture

Scripta is a synchronous AST interpreter written in safe Rust. Its execution
pipeline is **source → tokens → AST → evaluation**. Lexing and parsing finish
before execution begins; each phase preserves source spans for diagnostics.

## Crate boundaries

| Crate | Responsibility |
| --- | --- |
| `scr-diagnostics` | Source spans, positions, and diagnostic rendering |
| `scr-ast` | Syntax structures shared by the parser and evaluator |
| `scr-lexer` | Tokenization, escapes, and interpolation tokens |
| `scr-parser` | Statement and expression parsing, syntax checks, and recovery |
| `scr-runtime` | Values, lexical scopes, evaluation, objects, modules, and native interfaces |
| `scr-stdlib` | Registered automation APIs and their host policies |
| `scr-ext-sqlite` | SQLite backend and native database handles |
| `scr-web` | HTTP supervision, interpreter workers, and request lifecycle |
| `scr-cli` | Commands, shared pipeline, formatting, packages, and worker entry points |
| `scr-release` | Distribution packaging, checksums, notices, and package verification |

The runtime depends on the syntax crates, but stays independent of the standard
library and its backends. Standard-library modules and extensions register through
public native interfaces. SQLite is enabled by default in the CLI.

The `scripta` and compatibility `scr` binaries share `scr_cli::entry::main`.
Argument parsing performs no I/O; the command dispatcher owns file loading,
configuration, and process exit behavior.

## Parsing and diagnostics

The parser uses recursive descent for statements and Pratt parsing for
expressions. Recovery can report multiple errors, but never returns a successful
partial program. Syntax depth and complexity guards bound malformed input.

Spans are half-open UTF-8 byte ranges. Rendering maps them to filenames,
one-based lines, and Unicode scalar columns. Functions, modules, errors, and stack
frames retain source metadata so diagnostics remain accurate across file loads
and host invocations. Runtime traces record function names and call sites;
rendering shows the innermost frame first and redacts known credentials.

See the [language specification](language-spec.md) for syntax and error semantics.

## Runtime ownership and scopes

`Runtime` owns an arena of lexical scopes. Functions retain their declaration
scope, shared declaration metadata, source, and runtime identity. Calls create a
parameter scope followed by a body scope. Variable lookup follows lexical parents
but excludes globals across function boundaries; global function registrations
remain available. Defaults evaluate per call, in parameter order.

Values include strict scalars, UTF-8 strings, reference-counted immutable bytes,
owned lists and ordered maps, and reference-counted objects and callables.
Collections have value semantics; objects and callables have identity semantics.
Conversions are explicit.

Uncaptured scopes are released on exit. A tracing collector reclaims unreachable
captured scopes, including recursive closure and script-object cycles. Roots
include globals, active and suspended calls, cached modules, and externally held
callables and script objects. Collection runs at execution boundaries, selected
checkpoints, and before rejecting a scope allocation. Hosts can also call
`Runtime::collect_garbage()`. Opaque native ownership cycles require host cleanup.

Script classes and interfaces live in `objects.rs`. Instance fields use arena
scopes, so object cycles remain collectible. Ordinary instances store initialized
fields in declaration order. Checked slot hints fall back to name lookup for
uninitialized fields and exception storage. Bound methods retain their receiver
and lexical owner; direct method calls avoid a temporary bound callable while
preserving the same invocation semantics. Interface checks use declared type
identity and signature compatibility. See [objects and classes](objects.md).

Direct calls can use eight class-owned hints that retain no receivers or scopes.
Repeated misses disable the cache; native properties keep ordinary lookup.

## Execution hot paths and limits

The AST evaluator uses bounded internal optimizations: shared declarations,
reused call storage, small binding slots, verified map-position hints, numeric
plans, and buffer reuse. General evaluation handles unsupported cases. These
paths preserve value semantics, diagnostics, checkpoints, and failed assignments.

Small parameter scopes share immutable names with function declarations, falling
back to ordinary bindings when needed. Successful built-in type checks avoid
class lookup. Argument reuse retains at most 64 empty buffers of up to 16 slots.

Cleared scope storage and reserved parameter records share a 64-entry pool,
bounded to 32 retained collection slots per entry. Up to eight empty block
records remain reserved separately. Captured parameter scopes and those with
namespace storage or larger bindings follow normal release and tracing. Reserved
records have their parents and flags reset, retain no values or names, and do not
count as live scopes.

`RuntimeOptions` controls source size, recursion, instructions, logical value
bytes, live scopes, output, and cancellation. Calls have a hard ceiling of 64
active script/native frames. Logical budgets are not an aggregate heap or RSS
cap. Native operations have their own limits and timeouts; the interpreter has
no general wall-clock deadline.

Internal signals unwind returns, loop control, exits, exceptions, and failures.
Catchable errors retain their kind, span, and trace. Cancellation and fatal
execution limits bypass script catches and cleanup; Rust RAII owns host-resource
cleanup. See [resource behavior](standard-library.md#distribution-and-resource-behavior).

## Native APIs and I/O

`NativeFunction` pairs a typed signature with a Rust callback. Registration rejects
duplicate names; arguments are checked before invocation and results afterward.
`RuntimeContext` supplies metadata, runtime-local environment and working-directory
state, streams, permissions, cancellation, and the call span. `NativeObject`
provides checked host handles and properties.

Filesystem, process, HTTP, stream, database, crypto, and timezone backends belong
in the standard library or extension crates. They enforce capabilities, resource
ownership, closed state, and operation-specific limits. Filesystem root checks
reject stable path escapes but do not sandbox adversarial filesystem races.
Process handles own managed children and cleanup monitors rather than bare PIDs.

`NativeFunction::reentrant` and `Runtime::invoke` let hosts call scripts using the
same scopes, buffers, limits, and error signals. Execution APIs return outcomes
without terminating the host process. See [writing extensions](writing-extensions.md)
and the [standard-library reference](standard-library.md).

Output uses a generic per-execution buffer controller shared by echo, native
calls, and loaded modules. Pending bytes, nesting, and cumulative output are
bounded. Normal completion drains buffers; failures discard pending buffers.

## Source loading and developer tools

The runtime loader canonicalizes file identity, checks host policy, tracks active
loads, and preserves defining sources. Ordinary loads execute in the current
scope. Aliased loads create isolated roots, cached only after successful
initialization; aliases and cached modules participate in tracing. Isolated
modules can access registered natives without inheriting caller state. See
[local script loading](language-spec.md#local-script-loading).

The CLI shares lexing and parsing across execution, checking, and formatting.
Checking validates without running code. Formatting uses AST layout hints and
original tokens/comments, verifies token equivalence, and publishes validated
output atomically. Package installation and discovery stay in the CLI; execution
uses installed sources. See [packages](packages.md) and [development](development.md).

The CLI test runner discovers tests without executing them, then runs each test
in a managed process with bounded result files and runner-only native APIs.
The runtime supplies generic callback invocation. See [testing](testing.md).

## Web request hosting

`scripta serve` uses Hyper/Tokio for HTTP and a bounded queue of persistent
interpreter workers. Each worker caches the entry AST and creates a fresh runtime
per request. The supervisor owns admission, trusted proxy metadata, deadlines,
worker replacement, upload spools, and shutdown. Worker IPC uses bounded metadata
and body frames, separate from script output.

Request, response, upload, cookie, and session APIs are host-registered native
objects in `scr-stdlib::web`. Session commits and response metadata are coordinated
before output begins. Explicit response flushing uses bounded frames and
backpressure; successful handoff does not prove remote receipt. Workers are
stopped and reaped before their owned request files are removed.

Observability and development reloads belong to `scr-web`. Reloads stage a valid
source snapshot and replacement workers before draining the old generation;
failed reloads retain the old pool. See [web hosting](web-servers.md) for operational
limits, sessions, streaming, health checks, and development mode.
