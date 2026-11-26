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
| `scr-db` | Shared database registration, runtime-owned handles, and driver boundaries |
| `scr-ext-sqlite` | SQLite backend and database handles |
| `scr-ext-postgres` | PostgreSQL v3 transport, verified TLS, SCRAM, native prepared execution, typed fetching and transactions; optional CLI postgres feature |
| `scr-ext-mysql` | Verified-TLS MySQL connections, native prepared execution, typed fetching and transactions; optional CLI mysql feature |
| `scr-web` | HTTP supervision and interpreter workers |
| `scr-cli` | Commands, formatting, packages and worker entry points |
| `scr-release` | Distribution packaging and verification |

The runtime depends on syntax crates, not the standard library or its backends.
Extensions register through public native interfaces. The `scripta` and legacy
`scr` commands share one CLI entry point; SQLite is enabled by default.

The [database connector foundation](database-connectors.md) composes backend
descriptors through `scr-db`. The CLI always registers the shared Db namespace
and enables MySQL/PostgreSQL with its optional mysql/postgres features. One scr-cli extension
registrar covers checking, execution, embedding, test workers and web workers.
Verbose discovery reads the same compiled descriptor metadata without I/O.
The shared layer retains bounded error snapshots independently of backend lifetime;
Both drivers supply sanitized structured codes without a runtime-wide error redesign. SQLite
remains default-enabled and exposes its independent Sqlite namespace.

Package add/install/update checks optional `[extensions]` API/capability requirements
against the same registered extension metadata. Execution preflights the installed
graph before initialization and revalidates module loads. Lock format 1 remains
host-independent; native libraries are never downloaded or compiled by package
installation. `Runtime::add_module_policy` composes filesystem, watch and package
restrictions without replacing earlier checks. Web watch workers retain the
original entry path for project validation alongside the source snapshot boundary.

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
bodies, checked lookup hints and storage reuse accelerate common cases. Eligible
named scalar call graphs use statically indexed operand slots and explicit call
frames in reusable vectors, reserved before execution from plan and depth bounds.
Their inner calls omit lexical environments when scope budgets are unset;
budgeted calls preserve scope accounting. Ordinary evaluation handles unsupported
cases, including captures and arbitrary native callbacks.

Eligible self-recursion plans can prove integer arguments and returns and omit
redundant inner annotation checks. Outer arguments, checked arithmetic, and
execution limits retain their existing validation.

Prepared arithmetic and boolean operations use an inline success path; the
diagnostic converter is kept in a cold error path with the original source span.

Comprehensions share immutable binding names and recycle bounded uncaptured
scope storage. Read-only int/bool/null expressions can use compact plans and
direct input slots. Iterations retain scope accounting and instruction checks;
escaped closures retain independent bindings through ordinary evaluation.

Discarded string appends reuse the destination buffer when the suffix cannot
write script bindings. This includes non-reentrant native calls with similarly
restricted arguments, identified through the actual resolved callable. User
calls, reentrant callbacks, module aliases, and effectful arguments keep the
original prefix snapshot. Suffix evaluation, validation, and resource failures
precede the append commit.

An initialized scalar in an ordinary single-field instance has a fixed logical
footprint. Scalar replacements retain that payload size and the field's existing
checkpoint and annotation checks. Missing fields, inherited layouts, exceptions,
and values with variable payloads or shared handles use full object measurement.
Bounded straight-line scalar method plans use parameter/field slots for verified
recent runtime-created receivers, retaining ordinary field setters, annotations,
body scopes, and checkpoints. Unsupported states and explicit byte budgets fall back.

`RuntimeOptions` controls execution and logical resource budgets; these are not
an aggregate heap cap. Internal signals preserve exceptions, returns and exits.
Cancellation and fatal limits bypass script cleanup; Rust ownership handles
resource release. See [resource behavior](standard-library.md#distribution-and-resource-behavior).

## Native APIs and I/O

Typed native registration validates arguments and results. Runtime contexts
supply streams, permissions, cancellation and host state. Reentrant invocation
shares script scopes, budgets and error signals without terminating the host.
Invocation contexts expose a read-only logical byte ceiling for backend allocation
preflight. Eager collection callbacks use this public bridge; the runtime has no
collection-backend dependency. Key sorting computes each scalar key once, then
sorts indexes with a fallible stable merge; comparator sorting invokes scripts
through the same bounded bridge. Existing fixed-purpose helpers, comprehensions
and value-copy rules remain unchanged.
Backend-free execution-cleanup callbacks run after top-level outcomes and before
runtime teardown. Standard-library temporary resources and file locks opt into
this lifecycle through a bounded native registry, so cancellation does not leave
host-retained aliases open. Backend state, filesystem identity checks, removal,
and lock polling remain in scr-stdlib; existing resource lifetimes are unchanged.

Interactive process handles remain in scr-stdlib and reuse process-group/job
cleanup and bounded temporary-file output monitoring. Independently reopened
spool readers preserve child write cursors. A bounded stdin queue is serviced by
a limited writer thread; finite deadlines and bounded worker shutdown prevent
script coordination or escaped pipe readers from causing unbounded joins.
Existing staged-input/capture calls retain their behavior; stream APIs use the
public native interface and validate runtime ownership.

Buffered HTTP conveniences remain in scr-stdlib. A bounded multipart codec
constructs replayable bodies; explicit runtime-owned cookie jars use a bounded
exact-origin, host-only profile with atomic per-response updates. Opt-in retries
and manual redirects share a total deadline, bounded discarded bodies and strict
credential forwarding rules. Source streams remain non-replayable and reject
these options. Existing Rustls verification and download publication policies are
shared. Wire dates reuse locked httpdate; no runtime backend dependency is added.

Immutable full-URL values use a bounded WHATWG parser in scr-stdlib, reusing the
locked url/IDNA dependencies. Their parsing and reference resolution perform no
I/O and do not modify HTTP/Stream validation, permission, or TLS policy. Encoded
components and canonical text remain distinct from explicit query/form decoding.

Archive codecs live under scr-stdlib filesystem registration. Bounded ZIP catalog
preflight, raw TAR iteration, gzip-member limits, and checked codec I/O precede
publication of byte/list results. Filesystem extraction validates first and then
owns an exclusively created tree with identity-checked rollback. It never delegates
to permissive archive unpack helpers or applies archive links/permissions.

Configuration parsing remains in scr-stdlib. TOML uses the locked bounded-input
parser plus checked value conversion; YAML consumes low-level events without an
alias-expanding loader; INI has an explicit literal-string dialect. Shared limits
account values, map keys and UTF-8 payloads before growing returned structures.
No configuration format backend or dependency enters scr-runtime.

Date arithmetic remains in scr-stdlib: Jiff supplies civil/calendar operations
and bundled timezone resolution with explicit rejection of ambiguous destination
wall times. Chrono retains existing UTC timestamp validation for elapsed-second
addition and differences. A bounded, whole-second duration parser has no I/O or
new dependency; calendar units remain distinct from fixed elapsed durations.

OS, network and database backends belong in standard-library or extension crates.
They enforce capabilities, resource ownership and operation-specific limits.
Process stdin payloads are staged in bounded temporary regular files; stdout and
stderr capture use independent temporary files. The monitor retains child/group
cleanup, deadlines and cancellation without bidirectional pipe dependencies, and
charges captured payloads to logical value budgets before result allocation.
Script and native output share a host writer and cumulative byte accounting.
The runtime has no application buffer stack; HTTP hosts own response buffering
and transport flushing. See
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
