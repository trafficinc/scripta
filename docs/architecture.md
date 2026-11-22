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
| scr-ext-sqlite | SQLite connection policy, native database handles, and registration | runtime |
| scr-web | Bounded HTTP host, worker supervision, cached entry programs, framed worker IPC | runtime, stdlib, ast, lexer, parser |
| scr-cli | CLI commands, package management, web configuration, playground launching, shared pipeline, and diagnostics | Pipeline crates, web, SQLite extension (default-enabled feature) |
| scr-release | Maintainer archive/checksum/license verification and offline package smoke | None (standalone build tool) |

## Executable names

The primary Cargo binary is `scripta` and the compatibility binary is `scr`.
Both small entry points call `scr_cli::entry::main`, so arguments, runtime
configuration, diagnostics, and exit behavior share one implementation. Cargo's
`default-run` selects `scripta`. Release archives contain the primary executable
and an identical compatibility copy, covered by the release inventory.

## Public application interfaces

`scr_cli::args::parse` consumes arguments excluding the executable name and
returns `Result<Command, UsageError>`. `Command` represents file execution
(`Run`), source from stdin (`RunStdin`), `Check`, `Format`, `Help`, `CommandHelp`,
`Version`, and `VersionVerbose`. It also carries arguments for public `Serve`,
`Playground`, `Test`, and `Package` commands, plus internal `WebWorker` and
`TestWorker` entry points.
The package dispatcher handles `init`, `add`, `remove`, `install`, `update`,
`list`, `outdated`, and `search`; playground launching resolves the separate companion executable. Parsing performs no I/O
and never exits the process. `args::HELP` is the shared CLI usage text. File paths
preserve their native OS encoding. See [package management](packages.md),
[web hosting](web-servers.md), and [playground installation](install.md#optional-local-browser-playground).

Package discovery lives in `packages/index.rs`, with bounded TOML catalogs from
local files or temporary Git checkouts. Bare `add` materializes ordinary sources
in the manifest; execution never consults the index. `packages/commands.rs`
implements read-only graph/version reports and snapshot-preserving removal through
the shared installer transaction. Package identities support `owner/name`;
installed-lock ancestry walks the `.scripta/packages` boundary rather than assuming
one directory per package name. Namespaced packages use the reserved
`.scripta/packages/.namespaced/owner/name` layout, keeping an `owner` package
separate from `owner/name`. No runtime or standard-library dependency on the
package index is introduced.

`scr_cli::pipeline::check(source: &str)` returns
`Result<(), Vec<scr_diagnostics::Diagnostic>>` after lexing and complete statement
parsing. It reports all safely recoverable parser errors without evaluating the
program. `scr_cli::pipeline::run(source, output)` returns
`Result<u8, scr_runtime::RuntimeError>` and lexes, parses, then executes in a fresh
runtime with standard-library
functions registered. The caller supplies a `Write` destination. File loading
and exit codes belong to the executable, so embedding callers can provide their
own source and error handling.

`scr_cli::pipeline::run_with_input(source, output, options, input)` accepts an
owned buffered input reader. The CLI uses an empty reader after consuming stdin
as source so script input stays at EOF. File-backed execution retains process
stdin for pipeline records.

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
range checks belong to the parser. Comments and whitespace are consumed as trivia.
Execution, checking, formatting, stdin, includes, and web entries use the same lexer
without inserting source bytes or remapping positions. A single EOF token is
emitted at the original source's end.

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
Values include UTF-8 strings, immutable reference-counted byte slices, owned
lists/maps, and reference-counted native objects and callables. Strict equality is implemented by value for scalars/collections and
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
on all control-flow paths and their slots reused. Captured environments use a
small stop-the-world tracing collector. Globals and all active scopes (including
suspended callers) are roots; tracing follows lexical parents and callable values
inside lists/maps, variables, and function bindings. Unreachable captured scopes,
including recursive environment/function cycles, are dropped and slots reused.
A callable from one runtime cannot execute in another runtime.

Name resolution walks current lexical maps and returns the value with its scope,
so successful variable/function reads avoid hashing the same name a second time.
Assignment places borrow names from the live AST; overwriting a variable updates
its existing map entry, allocating a key only for a new binding. No binding-index
cache or identifier interner is introduced: includes, shadowing, host injection,
and isolated-module native fallback continue to observe current scope contents.

Released scope storage is reused through a best-effort runtime-local pool of at
most 64 empty scopes. A scope is eligible only when the sum of its five map/set
capacities is at most 32 slots (at most 2,048 retained capacity slots across the
pool). All values, function/class/alias bindings, module paths, parents, capture
flags, and function-boundary flags are cleared before pooling. Captured scopes
are eligible only after tracing proves them unreachable. Oversized scopes and
pool-allocation failures use ordinary destruction. The pool retains no graph
roots, does not count as live scopes, and is released with the runtime; these
capacity bounds are not an exact heap/RSS limit.

A weak user-function registry supports host/evaluator roots without changing the
public Value API: the collector counts callable references in every arena value,
then compares those counts with each function's Rc strong count, discounting its
own temporary upgrade. Any remaining reference conservatively roots the captured
environment. This protects host-held callbacks, expression temporaries, and
callbacks hidden in native objects or native function implementations. Native
property getters and callback bodies are never invoked during tracing. Opaque
native ownership cycles are not collected and can retain environments; hosts
must break those cycles themselves.

Collection runs at execute entry/exit, at statement/loop checkpoints after a
threshold of user-function allocations, and before a scope limit rejects an
allocation. The initial threshold is 256 function allocations and grows with the
surviving scope/function population to amortize scans. Hosts can also call
`Runtime::collect_garbage()`, which returns the number of reclaimed scopes.
Collection is synchronous; no bounded pause-time guarantee or hard RSS ceiling
is provided. Arena/allocator capacity can remain at its high-water mark.

Internal control-flow signals carry return, break, continue, exit, explicit throw,
and runtime failure through nested statements. Loop/function/catch boundaries
handle only their own signals. Explicit throws, TypeErrors, native exceptions,
property errors, and core IndexError/ArithmeticError exceptions are catchable.
Core operations return a typed error category independently of message wording;
the runtime attaches the original span/trace and redacts the diagnostic before
exposing a structured error object to a catch. Exception objects use the same
ScriptObject arena ownership as user classes; causes are hidden scope values
traced by the collector, while read-only metadata retains the original diagnostic.
Built-in exception class identities are cached per runtime. Exception-only
inheritance resolves lexical bases, guards cycles/depth, and retains ancestor
environments. Catch matching uses ancestry; rethrowing preserves the first
throw diagnostic and identity. Resource failures remain fatal,
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
limits. Optional checkpoint, logical-value byte, and live-scope budgets plus host
cancellation bound execution. These do not impose an aggregate allocation/RSS
ceiling or an interpreter-wide wall-clock deadline; individual native operations
such as HTTP requests and subprocesses have their own timeouts.


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
subprocesses. Local macOS results do not establish Windows/Linux validation.

The process backend uses the [process-wrap 8.2.2 API](https://docs.rs/crate/process-wrap/8.2.2)
with the standard-library frontend. Application-owned setup guards precede the
platform wrapper so job setup failures cannot abandon the direct child. Capture
is implemented in scr-stdlib; the backend's unbounded output-reading API is unused.


## JSON, HTTP, and source reuse

JSON conversion lives in scr-stdlib and rejects unsupported values before serde_json
serialization. HTTP uses a blocking ureq/Rustls client with explicit timeout,
redirect, and response-size limits. HttpResponse returns native callables for its
methods through `Value::native_function`, using the existing callable evaluator.
Responses retain raw bytes; default text mode validates UTF-8, and binary mode
leaves decoding explicit. Native objects can report retained payload size through
`NativeObject::payload_bytes`; HTTP uses this for body/header accounting.
Downloads use bounded buffers, filesystem policy checks, same-directory temporary
files, and exclusive publication after a successful transfer.

CSV combines a bounded, strict record scanner with the Rust `csv` parser/writer.
File readers retain descriptors and return complete records across line breaks.
Regex uses a small API backed by Rust's `regex` crate with fixed pattern/compiled
size limits. Its native registration closures share a runtime-local LRU cache
in `scr-stdlib`, bounded to 8 successful compilations and 256 KiB of pattern text.
The cache retains `Rc<Regex>` values without cloning engine scratch state; failed
compilations leave the cache unchanged. No backend cache state enters
`scr-runtime`. Explicit grapheme helpers use `unicode-segmentation`; existing scalar
length and substring operations are unchanged.

Local loading is a runtime statement. The loader canonicalizes identity, tracks
active files, consults a host policy, and evaluates each parsed file in the current
lexical scope unless `as Alias` requests an isolated lexical root. Unaliased once
completion belongs to the enclosing root; successful isolated instances are
cached by canonical entry path. Lexical alias bindings reference module scope IDs,
and qualified calls select functions directly from the target module scope.
Isolated roots have no caller parent; native registrations use a native-only
fallback. The tracing collector follows alias edges and treats cached module
roots as runtime-lifetime roots, including modules without functions. Aliased
`require` and `require_once` share the same canonical-path cache and publish an
alias only after successful initialization. Subsequent loads reuse that root.
Scope limits include cached module roots. SourceFile references on user functions,
runtime errors, and stack frames retain original file text for cross-file diagnostics and relative
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


## Execution hot paths and limits

Scripta executes through its AST evaluator in every feature configuration. The
evaluated bytecode prototype was removed; there is no engine-selection API or
bytecode build feature. Native registration and the public Value representation
retain their existing contracts.

The AST evaluator and owned public Value representation remain. Borrowed index
traversal avoids cloning an entire variable collection before selecting an item;
effectful indices retain snapshot evaluation. Discarded assignments move values,
and discarded variable-string appends reuse buffers for pure `$s .= suffix`
and `$s = $s . suffix` forms. Self-concatenation accepts grouping around the
concat or prefix, but requires that prefix to be exactly the destination variable.
The suffix is evaluated into an owned value before mutation, so self-reference
and independent string copies keep their value semantics. Checked byte growth
and fallible reservation precede the append; errors use the original concat span
for `=` and target span for `.=`. Used assignment results, effectful suffixes,
concat chains with a non-variable prefix, indexed targets, and properties retain
the general snapshot path. No checkpoints are added or removed. Indexed writes modify
owned roots while explicitly budgeted updates use a transactional candidate.
Ordered maps keep the public `Value::Map(Vec<(MapKey, Value)>)` representation.
Core indexed reads and updates compare borrowed typed keys, cloning string keys
only when inserting a new entry. The first map segment of a variable-rooted
index path can use runtime-local position hints for maps with 64–4,096 entries.
Hints own typed keys and positions, retain no values or graph roots, and are
checked against the actual current vector key before use. Misses always fall
back to normal lookup; a stale hint cannot establish absence. Whole-variable
writes, host `set_global`, and destructuring invalidate matching hints. Verification
also protects against reordered maps and reused scope IDs. Nested segments,
properties, small/oversized maps, and native helper lookups retain normal lookup.

The hint cache is bounded to 8 indexes, 4,096 indexed positions in total, and
256 KiB of UTF-8 string-key and variable-name bytes. Reads build indexes lazily;
writes only use existing hints, avoiding repeated builds during insertion-only
growth. Allocation failure declines the optimization. A newly built index may
briefly coexist with the bounded cache before eviction. These limits are not an
exact process heap cap. Insertion order, independent owned collection copies,
logical accounting, full validation, and checkpoint/cancellation semantics remain
unchanged.

No function declaration AST is cloned per call; the registration clone owns data
that can outlive the caller’s Program. Value validation is iterative and bounded
at depth 128; map duplicate validation uses typed hashed keys.

RuntimeOptions exposes source, instruction, per-value logical-byte, and live-scope
budgets. The tracing collector described above reclaims unreachable captured
environments during execution and at execution boundaries, including before a
scope allocation is rejected at its configured limit. Host-retained callables
remain roots; opaque native ownership cycles require host cleanup. Collection
does not enforce an aggregate RSS ceiling. Runtime error redaction is
separate from script exception values. Process waiters receive monitor completion
notifications while timeout/cancellation/capture checks remain bounded polling.
See [resource behavior](standard-library.md#distribution-and-resource-behavior)
for CLI configuration and logical accounting.

## Web request hosting

`scripta serve` validates a fixed entry script and starts the `scr-web` HTTP host.
The CLI merges an explicit optional TOML configuration with CLI overrides;
`scr-web::Config::validate` checks effective limits and storage paths without
creating resources. Check/print modes stop before host startup. The runtime's
`SCR_*` limits remain separate. Connection/header deadlines and the shared
shutdown drain budget are host configuration, not language execution limits.
On shutdown a watch notification invokes Hyper graceful shutdown for every
connection; a service-entry flag check rejects requests buffered across the
admission boundary. Admitted body receivers and queued jobs drain under one
shared deadline. At expiry connection tasks abort and worker loops receive a
forced-stop notification, explicitly kill/reap children, and then release owned
spools before persistent/temporary stores close. Repeated handled signals only
set the same shutdown flag. OS reaping/cleanup adds overhead to the work budget.
Hyper/Tokio own HTTP/1.1 framing and asynchronous network/process I/O. A bounded
queue dispatches requests to persistent interpreter processes, each with a cached
entry AST and a fresh runtime per request. Source snapshots are supplied by the
parent, including on worker replacement. Request/response native objects live in
`scr_stdlib::web`, explicitly registered by the host; the core runtime has no HTTP
backend. CLI-only executions do not register the Web module.

The HTTP supervisor retains each socket peer, compiles bounded explicit proxy
CIDRs/host allowlists once, and resolves trusted request metadata before receiving
bodies or enqueueing work. Only its selected X-Forwarded policy affects resolution;
raw headers remain unchanged. Typed `web::ConnectionInfo` travels in request IPC
and supplies four read-only request properties. Trust policy/parsing stays in
`scr-web`; the standard library accepts trusted embedding metadata and the runtime
has no proxy/network backend.

`Runtime::execute_program_with_source` executes a host-cached AST with retained
`Arc<SourceFile>` metadata. Hosts must match the AST to its source. It enforces
source limits and restores the previous source association after execution.

Worker pipes use bounded, length-prefixed JSON metadata and separate raw-body
frames. Script output goes to a bounded writer, never the protocol stream.
Worker-loop slots proactively observe idle child exit and unsolicited output,
terminate failed children, and recover without application dispatch. Per-slot
replacement backoff is bounded at five seconds and resets on a completed exchange
or observed 30-second survival, not on handshake alone. Optional request-count
recycling retires only after the full exchange. The accept loop reaps unexpected
worker-loop task completions and restores slots with bounded task-failure backoff.
Ready/recovering gauges distinguish usable leases from replacement waits;
shutdown cancels pending recovery while healthy workers drain admitted work.
The supervisor terminates/replaces workers on deadline or protocol failure;
worker input monitoring also exits when the parent pipe closes. Response metadata
and output are request-local, so errors can discard partial bodies before HTTP
headers are sent. See [web hosting](web-servers.md) for operational limits.

Cookie helpers and request cookie parsing live beside the Web native objects.
The host separately registers `scr_stdlib::web::session` with a dedicated local
store configuration. Every worker receives the same store path/limits through
its internal process arguments. Session state is request-owned; stable per-ID
file locks serialize same-session work, while a short store lock protects record
creation, reclamation, and atomic commits. Pending new records start revoked
until committed, so abandoned work can be reclaimed immediately. OS randomness
supplies IDs; typed tagged storage preserves Scripta values and map-key types.
Records are saved after successful execution/output validation, or by explicit
Session::close. Cookie/commit failures become generic HTTP errors before output.

`Html::escape` is a pure standard-library native function, available to both CLI
and web hosts through normal registration. It needs no parser/runtime/backend
changes: a sizing pass validates the exact expanded string budget before
allocation, and both passes check cancellation while preserving UTF-8 text.

`Password` uses RustCrypto Argon2id through the standard native API with a trusted
`StdlibOptions.password` policy. Complete PHC hashes are parsed and cost-bounded
before allocation. The host reserves the workspace fallibly, uses RAII zeroization,
and delegates digest comparison to RustCrypto's constant-time Output equality.
Native computation remains synchronous; the web supervisor can terminate its
worker at the request deadline. No crypto implementation/backend enters the core
runtime. Diagnostics distinguish qualified function references from credential
fields so Password:: calls retain useful errors without logging argument values.

Multipart parsing lives in `scr-web`, using Multer over a bounded disk reader.
The supervisor owns raw/extracted upload spools and charges a shared atomic disk
budget before writes. Worker request metadata references private spool basenames;
raw multipart bodies are not copied through IPC. `scr-stdlib::web::uploads` owns
only request handles, checked by an opaque runtime identity and a host lifetime
guard. The runtime exposes that identity through `RuntimeContext` without owning
any HTTP/filesystem backend. Request files are read in bounded chunks and are
materialized only by explicit `bytes()`/`text()` calls under value limits.

Saving uploads uses typed staging-control frames on the existing worker channel.
The worker checks the destination capability and filesystem roots, then asks the
supervisor for a staging path beside the destination. The supervisor registers
its cleanup guard before replying and charges its trusted file size. A worker
copies/syncs and publishes without overwriting; the supervisor retains cleanup
ownership until the worker finishes or is killed/reaped. This prevents orphaned
save-staging files when a request deadline terminates a worker. These internal
control messages do not introduce streaming responses or output flushing.

Application output buffering is generic runtime I/O, independent of HTTP.
`Runtime::execute` wraps the caller's `Write` in a per-execution buffer controller;
echo, native output, required modules, and script calls share it. `ScriptOutput`
extends `Write` with `OutputAction` control for standard-library native helpers.
`RuntimeContext.output` exposes this interface; hosts still pass ordinary writers
to execution. Pending bytes and nesting are bounded; cumulative generated-byte
accounting includes cleaned data and never counts buffer transfers twice. Normal
completion drains remaining application buffers; errors discard them. No HTTP
backend is introduced into `scr-runtime`.

The web worker's writer retains default response buffering. Its first explicit
flush seals sessions, freezes metadata, and sends a typed response start. Body
frames are bounded to 64 KiB and a one-slot supervisor-to-Hyper channel provides
backpressure. Flush acknowledgements confirm supervisor handoff, not remote
receipt. Completion closes the channel cleanly; failure sets a separate terminal
flag, so an error survives even when the data channel is full and the HTTP body
ends with a transport error. Receiver drop cancels the request; worker stop/reap
still precedes upload-spool cleanup. Deadlines cover native/pipe/channel blocking.

General streams live in `scr-stdlib::streams`, with safe standard-library file,
TCP, and UDP backends and the existing Ureq/Rustls HTTP backend. The runtime
contains only generic values/native interfaces; no network backend enters it.
`ByteStream` and `DatagramSocket` use actual native downcasts, opaque runtime
identity, shared closed state, and RAII descriptor/quota leases. A per-registration
handle count bounds all new stream kinds together. Existing line-stream APIs
remain separate and retain their established behavior.

Reads/writes are binary and bounded; copy uses at most 64 KiB working chunks and
finishes short writes. UDP has packet operations rather than a byte-stream facade,
counts the complete received packet, and exposes truncation. File modes perform
capability/root/kind/read-only checks before opening/truncating. HTTP responses
retain owned limited readers, status/headers, and an absolute request deadline;
request bodies are streamed from readable file handles without materialization
or replay. GET redirects and TLS policy match the buffered HTTP client. Socket
I/O polls cancellation under operation deadlines; blocking connect/HTTP/filesystem
calls retain backend limitations and the web host's process-level outer deadline.

The upload audit extends private spooling to every nonempty raw request body.
The same bounded IPC descriptor carries whether multipart parsing occurred;
JSON/form convenience methods preserve media-type checks before materialization.
`bodyStream()` and `UploadedFile::stream()` construct read-only request-input
backends, with weak state leases in an input scope. Ending a request closes those
states even when a host retains a native value. The stream registration shares
its config/quota through a thread-local weak index keyed by opaque runtime
identity; dead entries are pruned and carry no backend ownership cycle.
`Web::download` validates a readable file, response/buffer state, metadata, and
known output size before sending attachment headers; its body follows the same
bounded output channel as explicit flushing.

`scr-stdlib::streams::sockets` owns low-level sockets and shares the stream registry,
identity, quota leases, and TCP/UDP backends. Logical blocking operations use
nonblocking OS descriptors plus Mio readiness with cancellation checkpoints.
`Socket::poll` registers bounded temporary socket clones and returns unique ready
handles in caller order. Socket2 provides safe creation/bind/listen and supported
options, including explicit address-family and backlog policy. No unsafe code,
privileged raw IP access, or protocol/event-loop backend is added to the runtime.
Default CLI permissions include inbound bind/listen; web workers explicitly remove
those capabilities. Listener accept reserves quota before dequeuing a client.

HTTP observability lives in `scr-web`, outside the language runtime. A supervisor
observer owns fixed atomic counters, bounded generated IDs, and text/JSON event
serialization. Request guards and response-body wrappers account for queueing,
header readiness, body handoff/failure/drop, and cancellation with RAII. Worker IPC
carries generated IDs and bounded diagnostic/application-log frames, rather than
forwarding raw worker stderr. Host error records contain codes/spans without
request/source text. `WebRequest.requestId` is supplied by the host as ordinary
read-only request metadata.

An optional loopback-only management listener serves JSON liveness, readiness,
and counter snapshots independently of application routes/worker slots. It has
fixed connection/header/time bounds. Readiness uses startup/drain flags, known
worker leases, and admission capacity; probes do not initiate worker recovery or
check application dependencies. Body completion records mean handoff to Hyper,
not proof of remote receipt. See [hosting observability](web-servers.md#logs-and-health-checks).

Opt-in development polling and source snapshots live in `scr-web::dev`. Production
serving never starts that task. Bounded recursive scans compare source bytes and
debounce changes before validating/copying a private generation tree. Candidate
workers are staged before the old pool's connection/worker drain; a stable
application listener and session store survive generation changes. The CLI worker
registration installs a snapshot-root include policy through the existing public
runtime interface, preserving canonical include identity and fresh request state
without adding a runtime filesystem backend. Reload failure retains the old pool;
shutdown owns polling cancellation, staged children, and snapshot cleanup. See
[watch semantics and limits](web-servers.md#development-watch-mode).

## Timezone formatting

`scr-stdlib::date` keeps Unix milliseconds as the public timestamp representation.
Jiff resolves timezone rules; Chrono retains the existing strftime formatting,
RFC 3339 parsing, and UTC log/cookie behavior. `Date::format` adds an optional
string selector without adding native exports or runtime date types. Named zones
use `TimeZoneDatabase::bundled()` explicitly; `tzdb-bundle-always` embeds IANA rules
on every platform, including minimal CLI builds. Default Jiff features are disabled
so named formatting does not select a system database. `"local"` uses Jiff's
fallible host detection, its process-global cache, and host configuration. Runtime
`Env::set` does not mutate that process state.

The formatter obtains the offset and abbreviation at the supplied instant, then
uses a counting writer to validate the exact result size before allocating the
output. Both formatting passes check cancellation. Date backends stay out of
`scr-runtime`; no shell command, network lookup, or dynamic timezone plugin is
involved. The new dependency graph and timezone rules are covered by ordinary
release dependency notices and archive hashes.

## Script testing and host callback invocation

`testing.rs` owns the CLI test runner. `testing/discovery.rs` parses source without
execution, `testing/assertions.rs` registers runner-only `Assert`/`Test` APIs, and
`testing/worker.rs` initializes and invokes one test per process. Process groups
or Windows jobs and parent-owned temporary directories bound worker lifetime;
results use a separate bounded JSON file, never script stdout as a protocol.
Optional `[test]` manifest metadata is shared with package parsing and preserved
by package edits. See [testing](testing.md) for discovery, limits, and exit codes.

The runtime contains no testing backend. `NativeFunction::reentrant` receives an
`InvocationContext` that can invoke callable values with existing scopes, output
buffers, cancellation, stack limits, and exception semantics. `InvocationError`
retains the original signal and exposes catchable exception kinds, diagnostics,
explicit exits, and interruption. `Runtime::get_callable` and `Runtime::invoke`
support host calls without resetting the execution budget. Source-aware functions
retain their own diagnostics and relative module resolution. `execute_outcome`
and `execute_source_outcome` distinguish `Completed` from `Exited(0)`; existing
`execute`/`execute_source` keep their numeric exit-code contracts.

## Array expressions

The AST adds slice, spread, and comprehension nodes, plus membership as a
comparison operator. Parser extensions live in
`scr-parser/src/extensions.rs`; slices handle the lexer's existing `::` token
contextually without changing namespace tokenization. `scr-runtime/src/arrays.rs`
implements bounded output construction, snapshot comprehensions with per-iteration
lexical scopes, argument expansion, and atomic destructuring. Negative index
normalization is shared by reads and writes in `ops.rs`. See
[array expressions](arrays.md).

## Script objects

The AST includes construction, class declarations, and interface declarations. AST visitors, CLI
validation, and formatter layout include class methods and defaults.
`objects.rs` keeps class/interface metadata (declaration, lexical scope, source,
owner), caches resolved interface identities after successful conformance checks, creates instance field scopes, enforces class identity and field types,
and returns receiver-bound user callables. Runtime class metadata stores immutable
method declarations behind `Rc`; escaped bound methods share those declarations
while retaining distinct callable identities, receivers, lexical owners, and
source records. Direct property calls resolve script methods before arguments
and invoke the same call-frame implementation without allocating a temporary
bound callable or adding it to the weak function registry. Callable fields and
native properties use ordinary value invocation. A direct invocation retains its
receiver as an external object root and temporarily roots the declaring method's
lexical environment across argument evaluation and call entry, including inherited
methods whose lexical owner differs from the receiver's class. Those temporary
roots unwind on errors, interruption, and exit. Script instances use the existing
NativeObject value carrier with an internal checked type; ordinary native object
APIs remain unchanged. Fields are stored in the tracing arena, not an Rc-owned
field graph. Weak object tracking and receiver edges extend the collector's
existing external/internal-owner accounting. Class and object scope edges keep
module state alive; object cycles and method cycles remain collectible. Logical
budgets count declared fields and owned payload without recursively expanding
object-reference cycles. Interfaces share the class type namespace and metadata
carrier; their AST signatures use function declarations with empty bodies.
Entry interfaces are hoisted before classes. Class contracts resolve at the
original declaration execution position or before construction, allowing earlier
module requires to supply interface aliases. Conformance compares signature shape
and lexically resolved type identity; interface annotations check explicit
implementation identity rather than method presence. Retained interface metadata
adds defining-scope edges to tracing. See [objects and classes](objects.md).

## String interpolation

The lexer stores an interpolated string as one token containing decoded text and
embedded token streams with original source positions. Normal tokenization handles
nested quotes, comments, and balanced expression braces; nested interpolation is
bounded before recursion. The parser builds an `InterpolatedString` expression
and shares its syntax limits with embedded parsers. AST expression visitors include
all parts, so CLI validation also sees embedded function bodies. The formatter
keeps the original token text intact. The runtime evaluates parts in order,
accepts only strings and finite numeric/bool scalars, and checks result size and
cancellation before growing the output. Interpolation adds no native registration
or standard-library dependency. See [the language spec](language-spec.md#string-interpolation).
