# Architecture

Scripta is a synchronous AST interpreter written in safe Rust:
**source → tokens → AST → evaluation**. Parsing finishes before execution.

## Crate boundaries

| Crate | Responsibility |
| --- | --- |
| `scr-diagnostics` | Source spans and diagnostics |
| `scr-ast` | Syntax structures |
| `scr-lexer` | Tokenization |
| `scr-parser` | Parsing, syntax checks and recovery |
| `scr-runtime` | Evaluation, values, scopes, objects and native interfaces |
| `scr-stdlib` | Automation APIs and host policies |
| `scr-db` | Shared database API and runtime-owned handles |
| `scr-ext-sqlite` | SQLite backend |
| `scr-ext-mysql` | MySQL backend |
| `scr-ext-postgres` | PostgreSQL backend |
| `scr-web` | HTTP supervisor and interpreter workers |
| `scr-cli` | Commands, formatting, packages and worker entry points |
| `scr-release` | Distribution packaging and verification |

The runtime depends on syntax crates; standard-library and database backends
register through its public [native interface](writing-extensions.md).
The `scripta` and legacy `scr` commands share one CLI entry point.

## Runtime ownership

Each runtime owns its scopes and execution state. Functions retain lexical scopes
and runtime identity. Collections have value semantics; objects and callables have
identity. Shared collection storage and immutable map-key text preserve independent
snapshots on mutation.

Native callbacks share the active output, call stack and execution budgets.
Reentrant callbacks invoke scripts through InvocationContext, preserving exceptions
and control flow. Native resources use Rust ownership and explicit cleanup.

## Execution hot paths and limits

Specialized evaluation paths preserve ordinary type, error and limit semantics.
Source/value sizes, scope counts, recursion, output and execution checkpoints are
bounded by runtime/host settings. Logical value limits are not a process heap cap;
blocking native work needs cooperative cancellation and backend deadlines.

Tracing reclaims unreachable captured scopes and script-object cycles. Active
calls, cached modules and host-retained values remain roots. Opaque native
ownership cycles require host cleanup. See the [language specification](language-spec.md)
and [performance methodology](../benchmarks/README.md).

## Extensions and packages

One CLI registrar supplies extensions to execution, checking, embedding, test and
web workers. SQLite is enabled by default; MySQL and PostgreSQL are optional.
Discovery reads registered version/capability metadata without I/O. See
[database connectors](database-connectors.md).

[Packages](packages.md) validate native requirements during installation and
execution; they do not install native drivers. Lockfiles remain host-independent.
Module policies compose filesystem, package and development-watch restrictions.

## Web request hosting

`scripta serve` uses an asynchronous supervisor and a bounded worker pool.
Workers cache parsed entry scripts and create a fresh runtime per request.
The supervisor manages admission, deadlines, workers and request files; web APIs
register through the standard library. See [web hosting](web-servers.md).

Single literal-index assignments rooted in a variable may borrow their key from
syntax instead of allocating an owned path. Root resolution and compound-value
snapshots precede RHS evaluation; the write rechecks the current binding and map
hint afterward. Replacement validation, retained results and handle owners keep
the ordinary ordering. Byte-budgeted and unsupported targets use general places.
