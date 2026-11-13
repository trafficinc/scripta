# Development

Use the pinned toolchain selected in `rust-toolchain.toml`. All crates inherit
edition 2024 and a minimum Rust version of 1.85. From the workspace root:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run -p scr-cli -- run examples/variables.scr
cargo run -p scr-cli -- check examples/hello.scr
cargo run -p scr-cli -- fmt --check examples/hello.scr
```

Run format, lint, and test checks on each platform being validated. CLI exit codes
are 0 for success, 1 for I/O or language errors,
and 2 for incorrect command-line usage. `help` and `version` succeed; `--help`,
`-h`, and `--version` are aliases. `run <file>` executes; `<file>` is shorthand.
`check <file>` validates lexing and statement syntax, producing no output on
success. It collects recoverable parser errors and exits with code 1 if any are
found, including missing semicolons, legacy equality, and invalid control-flow
context. It does not resolve variables, enforce type annotations, or execute
native functions. The runtime executes the implemented statement grammar;
`check` validates it without running loops, functions, output, throws, or exits.
`run`, `check`, and `fmt` accept command-local `--help`/`-h`, and
`scr help run|check|fmt` prints the same command help with exit 0.
Both `run` and `check` accept `--` before a path. `scr run -` consumes bounded
UTF-8 script source from stdin through EOF; `scr run -- -` addresses a literal
file named `-`. Stdin source uses `<stdin>` in diagnostics and the working
directory for relative includes/packages. Subsequent script input is at EOF. No file-extension restriction
is imposed. Non-Unicode paths retain the operating system's native encoding.

Language and command-line integration tests live in `crates/scr-cli/tests` so
Cargo discovers them in this virtual workspace. Shared source fixtures live in
the root `tests/fixtures` directory. Root `tests/language` and `tests/integration`
directories describe test ownership and future organization; they are not
automatically discovered by Cargo.

Runtime support now includes arithmetic, lexical/block scopes, control flow,
functions/closures, owned collections, conversions, exit codes, and explicit
throw/catch. The typed native API exposes context, streams, environment snapshots,
permissions, named exceptions, and cancellation. Standard library registration
includes `str`, `int`, `float`, `bool`, `type`, `len`, and `sleep`. See the
[registration guide](standard-library.md#registering-rust-modules) for new Rust modules.
For a complete crate, optional Cargo feature, CLI wiring, and testing walkthrough,
see [Writing built-in extensions](writing-extensions.md).
The [SQLite extension](sqlite.md) is the first optional built-in crate; use
`--features sqlite` for CLI builds and `--all-features` for extension-inclusive
workspace validation. SQLite-enabled builds need a C compiler.
`File`, `Dir`, and `Path` provide filesystem operations and optional root policies;
filesystem tests live in `crates/scr-stdlib/tests` and use temporary directories.
Process/Env/Cli now support owned children, literal arguments, bounded output,
deadlines/cancellation, runtime-local environment values, options, and prompts.
Tests in scr-stdlib/tests and scr-cli/tests cover fixture children and CLI invocation
parity. JSON, verified HTTPS, bounded responses, and local script loading are covered
by loopback HTTP/TLS and temporary-file integration tests, with no public network
dependency. Future work includes interpolation, richer exception values,
collection helpers, broader resource limits, and release packaging. Add runnable examples
when each feature works; the fixtures in `tests/fixtures` exercise complete
statements, Hello World output, script exit codes, and runtime stack traces.

Local validation on macOS arm64 with Rust/Cargo 1.99.1 passed formatting,
Clippy with warnings denied, and workspace tests (including lexer, expression,
statement, recovery, runtime, conversions, and CLI coverage). These local results
do not establish Linux or Windows validation.


## Formatter and diagnostic tooling

`scr fmt [--check] <file>` and `scr fmt <file> --check` share the public
`scr_cli::formatter::format(source)` implementation. Formatting uses four spaces
per indentation level, puts each statement on its own line, normalizes operator/
comma spacing, and indents unbraced bodies without adding braces. Original token
spellings (including every string literal), comments, BOM/shebang, `<?scr`, and
optional `?>` are preserved. Whitespace outside literals/comments uses LF and one
final newline; existing blank-line runs are normalized. Interior multiline comment
and string bytes are retained exactly. Unsupported/malformed scripts are rejected
without any write. The formatted output is re-lexed to verify identical token kinds
and values before it can be saved. The file writer atomically replaces the
canonical target, preserving permissions and a symlink used to address that target.
`--check` never writes; a difference produces E0502 and exit 1.

`check` also rejects duplicate direct function declarations in one lexical body,
including anonymous functions inside expressions/defaults, and repeated parameter
names. Signature ordering is checked by the parser. Default expressions are not
evaluated by `check`. The formatter traverses anonymous bodies and defaults and
keeps a function literal's closing brace attached to its expression punctuation.
Separate lexical
blocks may declare the same name. Declarations on mutually exclusive/dynamic paths
are not exhaustively analyzed, nor are required files loaded. This is a declaration
and syntax check, not complete static type checking of a dynamically typed language.

| Diagnostic code | Category |
| --- | --- |
| E0001 | CLI usage (exit 2) |
| E0002 | Host file/stream setup I/O (exit 1) |
| E0101 | Lexical/tag/unsupported operator errors |
| E0201 | Syntax and invalid control flow |
| E0202 | Unsupported legacy equality (`==`, `!=`) |
| E0204 | Parser nesting/expression complexity limit |
| E0301 | Duplicate function declaration in one body |
| E0302 | Duplicate function parameter |
| E0401 | Runtime/native failure, with stack trace |
| E0501 | Formatter token-verification failure (no write) |
| E0502 | Source needs formatting in `fmt --check` |
| E0601 | Package command, manifest, resolution, or installation failure |

Lexical, syntax, declaration, formatter, and runtime failures exit 1. Success is 0;
explicit script `exit(code)` remains unchanged; cooperative interruption is 130.
Codes are stable categories; exact diagnostic prose is not a machine-readable API.
Excerpts use Unicode scalar columns, a tab rendered as one space, logical LF/CRLF/CR
lines, and a caret at EOF for missing tokens. Included syntax failures preserve
lexical/syntax codes and hints, distinct from runtime failures.

The parser bounds recursive syntax nesting at 64 and an expression at 256 syntax
nodes, reporting E0204 before recursive parsing or AST cleanup can exhaust the host
stack. Large expressions should use intermediate variables. These are parser
resource limits, independent of the runtime's call/include depth limit.

Formatter snapshots live in `crates/scr-cli/tests/snapshots/formatter`; they cover
comments, unbraced control flow, loops, Unicode, BOM/shebangs, CRLF literal content,
and closing tags. Tests compare execution before/after formatting and check
idempotence and no-write behavior. Snapshot bytes are exempt from Git newline
conversion. Proptest exercises 512 arbitrary-Unicode and token-noise cases per
parser property and 256 generated valid formatter cases. To reproduce a failure,
use Proptest's printed seed/regression case; its default regression files remain
supported. Targeted malformed cases include 10,000-level nesting and operator
chains. These checks run as part of the Cargo test suite; they improve robustness
without claiming exhaustive proof over all resource inputs.


## Performance and runtime safety

See [the Phase 13 report](performance.md) for actual release benchmarks, per-child
RSS, binary size, safety verification, and execution budget contracts. Run
`cargo bench -p scr-cli --bench interpreter --locked` for phase timings, and
`python3 benchmarks/run.py --scr target/release/scr --output benchmarks/results/local.json`
for checked Scripta/Bash/Python CLI fixtures. Workspace Rust forbids unsafe code
through inherited Cargo lints. Source excerpts now mask literal/comment contents;
error codes and original Unicode column positions are preserved.


## Distribution validation

Release profiles, the five native build targets, reproducible archive metadata,
checksum verification, and the test-before-support policy are documented in
[releases.md](releases.md). Windows builds must use an explicit target for static
CRT configuration. Native dist tests exercise the optimized executable; test
compilation alone is not a platform support claim. The installed executable does
not depend on the maintainer scr-release packaging tool. Phase 13 benchmark
measurements used the earlier default release profile and do not measure the
new Phase 14 LTO/distribution profile.


## Aliased requires

`require_once "helpers" as Helpers;` loads an isolated module and calls its
functions with `Helpers::name()`. The AST's `StatementKind::Require` includes an
optional `Binding` for the alias name/span. No lexer keyword or native export was
added (`as` and qualified-call syntax already exist). Formatter token verification
and syntax-only checking apply to aliases. Unaliased loading retains shared-scope
behavior; see the [full contract](packages.md#package-aliases-and-isolation).

Focused tests live in `crates/scr-cli/tests/module_aliases.rs`, alongside installed
package/version/integrity coverage in `packages.rs` and parser statement tests.


## Atomic file writes

`File::writeAtomic` uses sibling staging and tempfile's native replacement backend
in `scr-stdlib/src/atomic_writes.rs`. Production staging and publication paths have
private I/O seams for deterministic failure tests; no script fault-injection API
is exposed. Unit tests cover partial writes, commit/cancellation boundaries,
cleanup failures, and destination revalidation. Integration tests in
`crates/scr-stdlib/tests/atomic_writes.rs` cover the public API, policies,
permissions, concurrent readers, and a Windows delete-sharing failure fixture.
The report example is checked through the CLI release-example suite. See the
[public contract](standard-library.md#atomic-file-replacement) before changing
permissions, commit ordering, or durability behavior.
