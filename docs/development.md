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

Run format, lint, and test checks on each platform being validated. See the
[CLI reference](../README.md#cli-reference) for commands and exit codes, the
[language specification](language-spec.md) for evaluation and error semantics,
and [platform compatibility](compatibility.md) for supported targets.

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

Use the [standard-library reference](standard-library.md) for native APIs and
resource policies, and [Writing built-in extensions](writing-extensions.md) for
crate registration and CLI wiring. SQLite is enabled by default; see
[building with SQLite](sqlite.md#building-with-sqlite) for source requirements.
MySQL is opt-in through `--features mysql`; see [database connector builds and
discovery](database-connectors.md#building-and-identifying-mysql-support). Check
`version --verbose` for compiled versions/capabilities. CLI, embedding, checking,
test and web workers share registration in crates/scr-cli/src/extensions.rs.
PostgreSQL is optional through `--features postgres`; its [driver guide](postgres.md)
describes isolated SCRAM/TLS fixture tests and the common contract run under both drivers.
Test PostgreSQL-only source builds alongside the default/minimal/all-feature variants.
Add runnable examples and focused tests when behavior changes.

Native package requirements are covered by `crates/scr-cli/tests/package_extensions.rs`
and the manifest validator's unit tests. Test default, no-default-feature, MySQL-only
and all-feature builds. The explicit cross-binary test requires a MySQL-enabled
executable in SCR_EXTENSION_COMPATIBLE_BINARY and a minimal executable in
SCR_EXTENSION_INCOMPATIBLE_BINARY; run the ignored
native_requirements_are_revalidated_when_an_installed_project_moves_to_another_binary
test after building those variants. It uses disposable local projects, not a
running database. Never count that test as passed merely because a normal run
ignored it. See [package requirements](packages.md#native-extension-requirements).

Execution-scoped native resource coverage lives in
`crates/scr-runtime/tests/native_cleanup.rs` and
`crates/scr-stdlib/tests/temporary.rs`. The latter spawns its own Rust test
executable as a lock fixture, with pipe handshakes for cross-process shared and
exclusive contention; no public server or Python installation is needed. Tests
cover fatal/ordinary cleanup, retained and foreign aliases, filesystem replacement
refusal, permission failures with retry, cancellation while waiting, and the
64-resource boundary. Logical handle byte boundaries are unit-tested in
`crates/scr-stdlib/src/temporary.rs`. Execute
`examples/temporary-resources.scripta` with a controlled existing parent directory
when reviewing this API.

Full-URL behavior is covered by `crates/scr-stdlib/tests/full_url.rs`, alongside
the retained component/query/form tests in `tests/url.rs`. Native exact byte budgets
and cancellation checks live in `src/full_url.rs`; paths here are relative to
`crates/scr-stdlib`. Coverage includes IDNA, IPv4/IPv6, default/nondefault ports,
opaque and file URLs, credentials, encoded delimiters, duplicate queries,
reference resolution, sharing, and unchanged HTTP permission/credential policy.
`examples/url-handling.scripta` is a pure controlled-input review example.

Archive regression tests are in `crates/scr-stdlib/tests/archives.rs`. They use
local byte fixtures for traversal, duplicates, link/special types, forged ZIP
catalogs and sizes, TAR termination, gzip metadata and expansion bombs. Unit tests
in `src/archives.rs` (relative to scr-stdlib) cover exact logical output budgets,
cancellation after partial extraction and identity-checked rollback. The shared
filesystem/collection tests remain relevant. Execute the self-cleaning
`examples/archives.scripta` to validate file composition and both extraction paths.
Archive support uses the already locked ZIP/TAR/flate2 libraries with minimal
features; release packaging keeps its explicit stored-ZIP settings.

Configuration tests are in `crates/scr-stdlib/tests/configuration.rs`, with exact
logical output budgets and cancellation cases in `src/config.rs`. Explicit LF
and CRLF strings cover TOML/YAML/INI conversion, Unicode and literal strings;
malformed input, duplicate definitions, unsupported dates/tags/aliases, nesting,
value counts and numeric overflow preserve failed-assignment destinations.
`examples/configuration.scripta` exercises all formats without external inputs.
TOML reuses the locked parser. YAML uses yaml-rust2 event tokens with the encoding
feature disabled, never its alias-expanding document loader.

Date/duration tests are in `crates/scr-stdlib/tests/date_operations.rs`; native
cancellation and exact scalar-budget tests are in `src/date_operations.rs`.
They cover month-end/leap-day clamping, negative/pre-epoch arithmetic, DST gaps
and overlaps, non-hour transitions, skipped dates, duration syntax/precision,
and exact signed integer/timestamp/input limits. Existing timezone and logging
regressions remain in `tests/system_date_log.rs`. The controlled
`examples/date-arithmetic.scripta` requires no network or host timezone setting.

Interactive process regressions extend `crates/scr-stdlib/tests/process.rs`,
using the same portable child fixture for responses before EOF, large simultaneous
stdout/stderr plus input, closed/foreign handles, early exit, cancellation,
timeouts, limits and tree cleanup. Native queue/read budget, worker-permit and
spool cleanup checks live in `src/process/interactive.rs`. The CLI example test
executes `examples/process-interactive.scripta` against the controlled
`tests/fixtures/process-interactive.scripta` child; no shell, public endpoint or
external scripting runtime is required.

HTTP convenience integration tests are in `crates/scr-stdlib/tests/http_conveniences.rs`;
unit tests for cookie storage/budgets, multipart allocation/cancellation and retry
policy live under `src/http/`. Fixtures bind localhost only and verify binary
multipart framing, cookie path/origin scope, credential stripping, retry ceilings,
Retry-After, cancellation/deadlines, verified TLS and download cleanup. Existing
JSON/HTTP and binary-stream tests cover default behavior and non-replayable source
rules. `examples/http-conveniences.scripta` runs without network arguments and is
also executed against a controlled server by the CLI example suite.

## Local disk cleanup

From the workspace root, preview removable build outputs and Python caches:

```sh
python3 scripts/clean_local.py
python3 scripts/clean_local.py --apply
```

Requires Python 3.9+ and Git. The default targets only `target/` and `__pycache__/`
directories under the project's source, scripts, benchmarks, tests, examples,
and playground directories. Removing `target/` also removes locally built
executables; the next Cargo build regenerates them and takes longer. Global Cargo
caches, toolchains, and custom target directories outside `target/` are preserved.
Stop builds, tests, benchmark runs, and other writers before applying cleanup.

Optional categories require explicit flags; preview them before applying:

```sh
python3 scripts/clean_local.py --dist --benchmark-results
python3 scripts/clean_local.py --dist --benchmark-results --apply
```

`--dist` includes local release packages and validation evidence in `dist/`.
`--benchmark-results` includes local measurement records in `benchmarks/results/`;
it preserves benchmark fixtures, comparison work, and `benchmarks/evidence/`.
Back up any records you want to retain before selecting these categories.

The script locates the repository relative to itself, previews by default, and
checks all candidates before deletion. It refuses candidates containing tracked
files, nested Git metadata, symlinks, junctions, or special filesystem entries.
Sources, documentation, manifests, lockfiles, credentials, installed `.scripta/`
dependencies, and other untracked work outside these generated directories are
preserved. Size estimates sum file lengths; hard links, sparse files, compression,
and filesystem allocation can change the actual space recovered. Exit codes are
`0` for a successful preview/cleanup and `1` for failure. Failed cleanup can leave
earlier candidates removed; deletion is not rolled back. Do not run concurrently
with changes to the repository or cleanup directories.

Run the disposable-repository safety tests with:

```sh
python3 -B -m unittest discover -s scripts -p 'test_clean_local.py'
```

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

## Python tooling tests

Run the Python tooling's regression tests without building Rust or accessing
GitHub:

```sh
python3 -m unittest discover -s scripts -p 'test_*.py'
```

Text-editing tests use explicit LF/CRLF bytes and cover rollback and Git checkout
rules with `core.autocrlf=true`. Build inputs stay LF while byte-exact fixtures
and upstream notices retain their original bytes. Local success does not establish
correctness on other platforms; see the native validation guidance in
[AGENTS.md](../AGENTS.md#cross-platform-validation).

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
| E0202 | Reserved (formerly rejected `==` and `!=`; now strict equality aliases) |
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

## Benchmarks and runtime safety

The [performance table](performance.md) summarizes the PHP/Python/Scripta
comparison. Its [methodology](../benchmarks/comparison/README.md) describes
workloads, output checks, measurement, and regeneration of both published tables.
Run regression tests before timing, avoid concurrent builds/tests, and save a
new evidence file for each run.

The [benchmark guide](../benchmarks/README.md) also covers the Bash comparison,
interpreter stages, collection helpers, memory lifecycle, tracing collector, and
HTTP workloads. Benchmark timings are observations, not CI performance thresholds.

Runtime limits and redaction are documented in the
[standard-library reference](standard-library.md#distribution-and-resource-behavior);
collector roots and resource ownership are described in
[architecture](architecture.md#execution-hot-paths-and-limits).
