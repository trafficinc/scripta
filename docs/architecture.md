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

Reentrant native callbacks use the runtime-owned invocation bridge, preserving
the active output, lexical ownership, stack and execution budgets. Its
`InvocationContext::invoke_owned` accepts owned argument arrays and moves their
values into the existing reusable argument buffer; borrowed `invoke` copies its
argument slice. Both paths share validation and ordinary callable execution.
Collection backends stay in scr-stdlib and retain input snapshots before moving
callback arguments; this does not change script collection value semantics.

Repeated comparator inputs may use borrowed invocation argument holders. Only a
successful check of a small immutable data tree can be reused; mutable objects,
callables and byte-budgeted calls retain full validation. Rust borrows keep the
snapshot structure fixed, and callbacks still receive independent cloned values.
Holder state is local to that borrow, not keyed by recyclable addresses.
Comparator holders grow during the existing cooperative preflight walk, retaining
its checkpoint/prefix-limit order; quota-enabled, key-sort and scalar-first paths
keep their ordinary preflight. Callback results and retained-output measurements
remain fully checked.

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





## Web request hosting

`scripta serve` uses an asynchronous supervisor and a bounded interpreter worker
pool. Workers cache parsed entry scripts and create a fresh runtime per request.
The supervisor owns admission, deadlines, worker lifecycle and request files;
web APIs register through the standard library. See [web hosting](web-servers.md).
