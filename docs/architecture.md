# Architecture

The pipeline is source → tokens → AST → sequential evaluation. Every phase
returns structured diagnostics with UTF-8 byte spans. The CLI renders those
locations as a filename, one-based line, and one-based character column.

| Crate | Responsibility | Workspace dependencies |
| --- | --- | --- |
| scr-diagnostics | Shared source spans and diagnostics | None |
| scr-ast | Syntax structures | diagnostics |
| scr-lexer | Source to tokens; tags and escapes | diagnostics |
| scr-parser | Tokens to AST; precedence and syntax errors | ast, diagnostics, lexer |
| scr-runtime | Values, environment, evaluation, native registry | ast, diagnostics, lexer, parser |
| scr-stdlib | Native function implementation and registration | runtime |
| scr-cli | CLI arguments, file loading, shared pipeline, stdout/stderr, exit codes | All pipeline crates |
| scr-release | Maintainer archive/checksum/license verification and offline package smoke | None (standalone build tool) |

## Public application interfaces

`scr_cli::args::parse` consumes arguments excluding the executable name and
returns `Result<Command, UsageError>`. `Command` has `Help`, `Version`,
`Run { path: PathBuf, arguments: Vec<OsString> }`, `Check(PathBuf)`, and
`Format { path: PathBuf, check: bool }` variants. Parsing performs no I/O and never
exits the process. `args::HELP` is the shared CLI usage text. File paths preserve
their native OS encoding.

`scr_cli::pipeline::check(source: &str)` returns
`Result<(), Vec<scr_diagnostics::Diagnostic>>` after lexing and complete statement
parsing. It reports all safely recoverable parser errors without evaluating the
program. `scr_cli::pipeline::run(source, output)` returns
`Result<u8, scr_runtime::RuntimeError>` and lexes, parses, then executes in a fresh
runtime with standard-library
functions registered. The caller supplies a `Write` destination. File loading
and exit codes belong to the executable, so embedding callers can provide their
own source and error handling.

The existing crate-root interfaces remain public: `scr_lexer::lex`,
`scr_parser::{parse, parse_with_diagnostics, parse_expression}`,
`scr_runtime::{Runtime, Value, Callable, MapKey, NativeObject, NativeFunction, RuntimeError, StackFrame}`,
`scr_stdlib::register`, and the AST types. Shared errors are
`scr_diagnostics::Diagnostic { message, span, code, help }`; `Span { start, end }` uses a
half-open byte range. `SourcePosition` adds byte offsets and one-based line and
column coordinates, with `SourcePosition::at` mapping offsets consistently with
the lexer (Unicode scalars, CR/LF/CRLF, and a zero-width leading BOM).
Diagnostics implement `Display` and `Error`, and `render`
adds filename, line, and column information. Argument errors use `UsageError`
because they have no source location.

`scr_lexer::lex(&str)` returns `Result<Vec<Token>, LexerError>`. Tokens and lexical
errors include byte spans plus start/end positions in the original source.
`LexerErrorKind` supports structured handling, and `LexerError` converts to the
shared `Diagnostic`. Numeric tokens retain their source spelling; parsing and
range checks belong to the parser. Tags, comments, and whitespace are consumed
as trivia. A single EOF token is emitted at the original source's end.

Statements use `Statement { kind: StatementKind, span }`; their ranges include
terminating semicolons or complete bodies. Blocks, variable bindings, parameter
annotations, function names, and catch clauses also retain source spans. The
Parameter AST includes `default: Option<Expr>` and `variadic: bool`; embedders
constructing AST parameters must supply `None` and `false` for ordinary required
parameters. Anonymous literals use `ExprKind::Function` with a boxed declaration
and the diagnostic name `<anonymous>`, without registering a named function.
Signature ordering is checked by the parser and defensively by the runtime.
The lexer recognizes a distinct Ellipsis token for variadic parameters. The
parser uses recursive descent for statements and Pratt parsing for expressions.
`parse` returns the first diagnostic; `parse_with_diagnostics` returns all safely
recoverable diagnostics. Recovery discards the rest of an already diagnosed
statement to a semicolon, closing block, or following statement boundary. A
program with any parser errors never becomes a successful partial AST. Loop
context resets at each function boundary; return requires a function context.

The runtime uses the lexer/parser for local required scripts and remains independent
of the standard library. Hosts can register
native functions and supply any `Write` output destination. Native failures become
diagnostics at the call site; output failures become diagnostics at the echo
expression. Runtime errors stop execution and preserve output already written.
Lexing and parsing complete before execution starts.

The evaluator walks owned AST values using safe Rust (`unsafe_code` is forbidden).
Values include owned lists/maps plus reference-counted native objects and
callables. Strict equality is implemented by value for scalars/collections and
by identity for native objects/callables. Arithmetic, ordering, and conditions
reject implicit conversions.

The runtime owns an arena of lexical environments. Parent IDs model lexical
scope; user function values retain their declaration environment ID and runtime
identity. First assignment creates a local binding unless a lexical ancestor
already contains that name. Function call scopes mark a variable-lookup boundary
that excludes globals while retaining access to captured enclosing function locals.
Function lookup still reaches registered global functions. Parameters, foreach
bindings, and catch bindings explicitly shadow outer names. Missing defaults
evaluate per call in parameter order in that parameter
scope, with the definition source active. Variadic annotations check elements,
then the collected list is validated against depth/value budgets. Anonymous
and named nested functions share capture, ownership, and source retention rules;
they do not acquire access to global variables. Uncaptured scopes are released
on all control-flow paths and their slots reused. Captured environments are conservatively retained
until the runtime is dropped, avoiding reference-counted environment/function
cycles. A callable from one runtime cannot execute in another runtime.

Internal control-flow signals carry return, break, continue, exit, explicit throw,
and runtime failure through nested statements. Loop/function/catch boundaries
handle only their own signals. Explicit throws, TypeErrors, native exceptions,
property errors, and core IndexError/ArithmeticError exceptions are catchable.
Core operations return a typed error category independently of message wording;
the runtime attaches the original span/trace and redacts the diagnostic before
exposing a core error's message to a catch. Resource failures remain fatal,
separate from recoverable data errors. Other runtime failures and host
interruption are reported to the host.
Top-level declarations are registered in a first pass before executing other
statements. RuntimeOptions configures recursion depth, cancellation, and an
optional checkpoint budget; loop back edges check interruption even with empty bodies. `Runtime::execute(&Program, &mut impl Write)` returns
`Result<u8, RuntimeError>` without terminating the process. `get_global` and
`set_global` allow hosts to inspect or inject values; injected and native-returned
values are checked for finite floats and valid maps.

`RuntimeError` exposes `message`, `span`, and `trace: Vec<StackFrame>`; frames
record function names and call-site spans. Rendering prints innermost frames
first. Syntax diagnostics convert into runtime errors with no call frames at the
application boundary. Native/output failures preserve active script frames.
`NativeFunction` contains a typed positional signature and a context-aware Rust
closure returning `NativeResult<Value>`. Runtime registration rejects duplicate
names; signature checks run before callbacks and return checks after them.
`RuntimeContext` borrows host metadata, a mutable environment snapshot, input/output/error
writers, permissions, cancellation state, and the original call span. Runtime owns
the settings through `RuntimeOptions.context`; the CLI fills script metadata.
`NativeError` carries an exception name and message; Rust I/O errors convert to
`IoError` and acquire source spans/stack frames at the native-call boundary.
`NativeObject` exposes a host type name and property getter/setter; setters default
to read-only errors. Property failures become `PropertyError`. Standard library
modules depend on this public runtime API; the runtime never imports modules or
matches their names. See [registration guide](standard-library.md#registering-rust-modules).

Calls are limited to 64 active script/native frames, with configurable lower
limits. Optional checkpoint budgets and host cancellation bound execution;
allocation and wall-clock limits remain future work.


## Filesystem modules

`scr-stdlib::filesystem` implements `File` and `Dir` using standard Rust filesystem
APIs; `paths` provides native-platform lexical `Path` helpers. Modules register
through the typed native API. The interpreter contains no filesystem dispatch.
`NativeSignature::with_variadic` supports typed trailing positional arguments for
`Path::join`. The runtime context lends a mutable working-directory PathBuf so
`Dir::change` updates only runtime state, with no process-wide chdir.

`FilesystemPolicy` is captured by registered callbacks. Optional canonical roots
and read/write flags combine with runtime capabilities. Default registration
allows filesystem access; explicit policy registration supports embedding limits.
Canonical ancestor checks reject stable symlink/traversal escapes but do not claim
to sandbox adversarial filesystem races. Listings return sorted, UTF-8, read-only
DirEntry snapshots and skip links; recursive deletion is not exposed. Filesystem
integration tests use temporary directories and platform-gated edge cases.


## Process, environment, and CLI modules

Process/Env/Cli register through the native API in scr-stdlib. Typed optional
native parameters support default argument lists and options maps. Env mutates
only the runtime's OS-string environment map; children receive an explicit cleared
and rebuilt environment. Cli reads only script arguments supplied by the parser.
Prompt input is an injectable BufRead; no global environment mutation occurs.

Each child is created with process-wrap (Unix groups or Windows jobs) and has a
setup-failure cleanup guard. A monitor owns the managed child and temporary
capture files and enforces deadline, cancellation, and combined output size.
Capture has no pipe/EOF dependency. Arc/Mutex state publishes immutable results;
script handles stay in the synchronous Rc-based runtime and safely downcast
through NativeObject::as_any. Wait/poll/kill operate on owned handles, never PIDs.
Last-handle drop requests cleanup and joins the monitor. Normal run also uses
this owned path. StdlibOptions configures filesystem policy and process defaults.

CLI signal handling sets the same cancellation flag used by the runtime and
monitors; interrupted execution reports 130 after dropping runtime resources.
Platform-specific child behavior and limitations are documented in the
[standard library](standard-library.md#processes). Integration tests use fixture
executables, bounded temporary capture, injected input/environment, and real CLI
subprocesses. Local macOS results do not establish Windows/Linux CI success.

The process backend uses the [process-wrap 8.2.2 API](https://docs.rs/crate/process-wrap/8.2.2)
with the standard-library frontend. Application-owned setup guards precede the
platform wrapper so job setup failures cannot abandon the direct child. Capture
is implemented in scr-stdlib; the backend's unbounded output-reading API is unused.


## JSON, HTTP, and source reuse

JSON conversion lives in scr-stdlib and rejects unsupported values before serde_json
serialization. HTTP uses a blocking ureq/Rustls client with explicit timeout,
redirect, and response-size limits. HttpResponse returns native callables for its
methods through `Value::native_function`, using the existing callable evaluator.

Local loading is a runtime statement. The loader canonicalizes identity, tracks
active/completed files, consults a host policy, and evaluates each parsed file in
the current lexical scope. SourceFile references on user functions, runtime errors,
and stack frames retain original file text for cross-file diagnostics and relative
requires after module initialization. Internal runtime error payloads are boxed to
keep the recursive interpreter's stack bounded as diagnostic metadata grows.


## Developer tooling

The CLI validation pass walks a parsed program to identify duplicate direct
function declarations per lexical body and duplicate parameters without evaluating
code or resolving dynamic types. The formatter builds layout hints from the AST,
then copies source token/comment slices while changing whitespace. Re-lexing
verifies token equivalence, and an atomic file writer saves only validated output.
`--check` compares bytes and never invokes the writer.

Diagnostics render code categories, logical source lines, Unicode scalar columns,
carets, and optional help. RuntimeError retains the diagnostic code/help in addition
to file metadata and stack frames, so required-file syntax failures keep their
original phase category. Parser depth/complexity guards and property tests bound
common stack-exhaustion paths on malformed source.


## Measured execution hot paths and limits

The AST evaluator and owned public Value representation remain. Borrowed index
traversal avoids cloning an entire variable collection before selecting an item;
effectful indices retain snapshot evaluation. Discarded assignments move values,
and pure string concatenation assignments reuse buffers. Indexed writes modify
owned roots while explicitly budgeted updates use a transactional candidate.
No function declaration AST is cloned per call; the registration clone owns data
that can outlive the caller’s Program. Value validation is iterative and bounded
at depth 128; map duplicate validation uses typed hashed keys.

RuntimeOptions exposes source, instruction, per-value logical-byte, and live-scope
budgets. Retained closure environments are still released at runtime drop, with
a configurable ceiling rather than a new collector. Runtime error redaction is
separate from script exception values. Process waiters receive monitor completion
notifications while timeout/cancellation/capture checks remain bounded polling.
See [measurements and safety details](performance.md).
