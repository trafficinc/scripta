# Development

Use the pinned toolchain selected in `rust-toolchain.toml`. All crates inherit
edition 2024 and a minimum Rust version of 1.85. From the workspace root:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run -p scr-cli -- run examples/variables.scripta
cargo run -p scr-cli -- check examples/hello.scripta
cargo run -p scr-cli -- fmt --check examples/hello.scripta
```

The primary command is `scripta`; Cargo also builds `scr` for compatibility.
Use `.scripta` for source files (`.scta` shorthand and legacy `.scr` also work).
Scripts start directly with code. The benchmark driver accepts `--scripta` and
legacy `--scr`; the separate playground launcher retains its `--scr` protocol.

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
`scripta help run|check|fmt` prints the same command help with exit 0.
Both `run` and `check` accept `--` before a path. `scripta run -` consumes bounded
UTF-8 script source from stdin through EOF; `scripta run -- -` addresses a literal
file named `-`. Stdin source uses `<stdin>` in diagnostics and the working
directory for relative includes/packages. Subsequent script input is at EOF. No file-extension restriction
is imposed. Non-Unicode paths retain the operating system's native encoding.

Interface contracts and composition are covered in
`crates/scr-cli/tests/interfaces.rs`, including module identity, signature failures,
field assignment, and tracing of interface-typed object cycles. Run the
self-contained `examples/interfaces.scripta` to exercise interchangeable notifiers.

`crates/scr-cli/tests/exceptions.rs` also checks classes, interpolation, and
structured errors together: method throws, ancestor catches, explicit causes,
rethrow identity and traces, failed assignments, and module source locations after
garbage collection. The combined cases exercise LF and CRLF source; the formatter
case also executes the formatted output.

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
The [SQLite module](sqlite.md) is enabled by default in the CLI. Building from
source needs a C compiler for bundled SQLite; downloaded executables do not.
Use `-p scr-cli --no-default-features` for a minimal source build. Embedders
continue to register `scr-ext-sqlite` explicitly.
`File`, `Dir`, and `Path` provide filesystem operations and optional root policies;
filesystem tests live in `crates/scr-stdlib/tests` and use temporary directories.
Process/Env/Cli now support owned children, literal arguments, bounded output,
deadlines/cancellation, runtime-local environment values, options, and prompts.
Tests in scr-stdlib/tests and scr-cli/tests cover fixture children and CLI invocation
parity. JSON, verified HTTPS, bounded responses, and local script loading are covered
by loopback HTTP/TLS and temporary-file integration tests, with no public network
dependency. List and Map helpers, logical value/scope/step budgets, and native
release packaging and structured exceptions are implemented. Callback-based
collection transforms and recursive merge variants remain unimplemented. Add
runnable examples when each feature works; the fixtures in `tests/fixtures` exercise complete
statements, Hello World output, script exit codes, and runtime stack traces.

## Local secrets and personal information scan

Install [Gitleaks](https://github.com/gitleaks/gitleaks) outside this repository
(`brew install gitleaks` on macOS), then run:

```sh
python3 scripts/scan_sensitive.py
```

The wrapper requires Python 3.9+ and Gitleaks 8.25+ (tested with 8.30.1).
Use `--gitleaks /path/to/gitleaks` or `GITLEAKS_BIN` for a custom installation.
If Gitleaks is absent from `PATH`, the wrapper also checks the ignored
`.security-tools/gitleaks` (`gitleaks.exe` on Windows). Do not commit the binary.
It runs locally, without uploading source or findings. The default scans current
files, including untracked and Git-ignored files such as `.env`, and patch history
on all local Git refs. Use `--scope files` or `--scope history` for one scan, or
`--root /path/to/project` to scan another Scripta project using these rules.

`.gitleaks.toml` extends the built-in secret rules with email address, labeled
phone number, and US Social Security number patterns. Personal-information matches
need human review: public contact information, licenses, examples, and test data
can match. Names, postal addresses, other countries' IDs, and arbitrary passwords
are not comprehensively detected. This is pattern scanning, not a security audit.
Gitleaks' built-in exclusions still apply; generated `target`, `dist`, local
scanner/report directories, `.scripta`, `.venv`, and Python caches are also excluded.
Symlinks are not followed, archives are not unpacked, and unreachable/deleted Git
objects and remote refs that have not been fetched are outside this scan.
Inline `gitleaks:allow` comments and local fingerprint ignore files do not suppress
this wrapper's results. No source fixtures are explicitly exempted.

The ignored `.security-reports/findings.json` contains only rule IDs, relative file
paths, line numbers, scan scopes, and commit hashes, plus scanner version and scopes.
Matched values, source lines, commit messages, and author identities are omitted.
Exit codes: `0` no findings, `1` potential findings to review, `2` scan failure.
Failed scans leave any earlier report unchanged; do not mistake it for fresh results.
Keep any optional local executable downloads under the ignored `.security-tools/`
directory; neither executables nor reports belong in Git. The script never downloads
or installs tools automatically.

Run the scanner regression tests with the executable installed:

```sh
python3 -m unittest discover -s scripts -p 'test_scan_sensitive.py'
```

## Releases

Use [the release procedure](releases.md) for version preparation, publishing,
promotion, and local packaging. Platform targets and OS baselines are listed in
[compatibility](compatibility.md).

Run the release tooling's regression tests without building Rust or accessing
GitHub:

```sh
python3 -m unittest discover -s scripts -p 'test_*.py'
```

These tests use explicit LF/CRLF bytes to check version edits and rollback,
validation-log parsing, and Git checkout rules with `core.autocrlf=true`.
Release inputs must stay LF while byte-exact fixtures and upstream notices retain
their original bytes. The native build workflow runs this suite on every release
platform. Passing locally does not establish Windows support; native Windows
tests, packaging, and extracted-package smoke validation remain pending until
that platform's build succeeds.

## Formatter and diagnostic tooling

`scripta fmt [--check] <file>` and `scripta fmt <file> --check` share the public
`scr_cli::formatter::format(source)` implementation. Formatting uses four spaces
per indentation level, puts each statement on its own line, normalizes operator/
comma spacing, and indents unbraced bodies without adding braces. Original token
spellings (including every string literal), comments, and BOM/shebang are preserved.
Whitespace outside literals/comments uses LF and one
final newline; existing blank-line runs are normalized. Interior multiline comment
and string bytes are retained exactly. Unsupported/malformed scripts are rejected
without any write. The formatted output is re-lexed to verify identical token kinds
and values before it can be saved. The file writer atomically replaces the
canonical target, preserving permissions and a symlink used to address that target.
`--check` never writes; a difference produces E0502 and exit 1.

Git attributes keep `.scripta`, `.scta`, and legacy `.scr` source files at LF
on every platform, including Windows checkouts with `core.autocrlf=true`.
Formatter snapshots and benchmark fixtures override text conversion to preserve
their exact mixed newline bytes.

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
comments, unbraced control flow, loops, Unicode, BOM/shebangs, and CRLF literal
content. Tests compare execution before/after formatting and check
idempotence and no-write behavior. Snapshot bytes are exempt from Git newline
conversion. Proptest exercises 512 arbitrary-Unicode and token-noise cases per
parser property and 256 generated valid formatter cases. To reproduce a failure,
use Proptest's printed seed/regression case; its default regression files remain
supported. Targeted malformed cases include 10,000-level nesting and operator
chains. These checks run as part of the Cargo test suite; they improve robustness
without claiming exhaustive proof over all resource inputs.


Library unit fixtures that spawn child processes share the test-only
`DESCRIPTOR_LIFECYCLE_TEST` guard with descriptor-close/lock probes. A forked
child can otherwise keep a duplicate file description and lock alive briefly,
even when descriptors are marked close-on-exec. Keep the close assertions
strict and isolate the fixtures. Process cwd fixtures canonicalize their
reported paths so Windows short paths and extended-length paths compare by
directory identity rather than spelling.

## Performance and runtime safety

See [the Phase 13 report](performance.md) for actual release benchmarks, per-child
RSS, binary size, safety verification, and execution budget contracts. Run
`cargo bench -p scr-cli --bench interpreter --locked` for phase timings, and
`python3 benchmarks/run.py --scripta target/release/scripta --output benchmarks/results/local.json`
for checked Scripta/Bash/Python CLI fixtures. Workspace Rust forbids unsafe code
through inherited Cargo lints. Source excerpts now mask literal/comment contents;
error codes and original Unicode column positions are preserved.

Run `cargo bench -p scr-stdlib --bench collections --locked --offline` to compare
map helpers at 128, 512, and 2048 entries, including mixed integer/string keys.
The dependency-free harness reports CSV medians from nine measured runs after
two warmups and checks result lengths. Parsing, registration, and fixture setup
are excluded; argument copying, runtime validation, and result assignment are
included. These are end-to-end helper timings, not isolated native-call timings
or CI performance thresholds. Map key operations use temporary borrowed-key
hash indexes and preserve ordered output vectors. Value-based comparisons still
use repeated searches and can take quadratic time: strict value equality
supports nested collections and identity-based objects/callables.

The HTTP benchmark driver and workload are described in
[benchmarks/serve.md](../benchmarks/serve.md). Run benchmarks separately from
builds and tests to avoid contention.
