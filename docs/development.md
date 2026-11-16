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
Use `<?scripta` for new scripts; legacy `<?scr` remains accepted. The formatter
preserves the existing tag. The benchmark driver accepts `--scripta` and
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
release packaging are implemented. Interpolation, richer exception values,
callback-based collection transforms, and recursive merge variants remain
unimplemented. Add runnable examples
when each feature works; the fixtures in `tests/fixtures` exercise complete
statements, Hello World output, script exit codes, and runtime stack traces.

Local validation on macOS arm64 with Rust/Cargo 1.99.1 passed formatting,
Clippy with warnings denied, and workspace tests (including lexer, expression,
statement, recovery, runtime, conversions, and CLI coverage). These local results
do not establish Linux or Windows validation.

## Tag-triggered builds and releases

[Native builds and releases](../.github/workflows/native-validation.yml) runs on
**every tag push**, including validation/prerelease tags. Branch pushes and pull
requests do not trigger it. The tagged commit must contain the workflow and its
[Python validation runner](../scripts/validate_native.py).

Four native jobs use Ubuntu 22.04 x86-64, Ubuntu 22.04 ARM64, Windows Server 2022
x86-64, and macOS 15 Apple Silicon. No Intel macOS job is included. Each installs
the pinned compiler, verifies its host target, checks formatting and the Python
validation/publication tests, and runs default/all-feature workspace tests in
both debug and dist profiles. Each profile also tests the minimal source build
with `--no-default-features --test sqlite_extension` to verify SQLite absence.
All-target/all-feature Clippy denies warnings.
Explicit native targets preserve Windows static CRT settings; macOS builds set
`MACOSX_DEPLOYMENT_TARGET=15.0`.

Each job builds one standard executable with SQLite, inspects dependencies, verifies
archives/checksums, compares identical-input repacks, and smoke-tests extracted
packages. Full evidence is retained as Actions artifacts for seven days, including
failure logs. Jobs do not cancel one another.

After **all four jobs succeed**, a separate publisher job downloads this run's
current target artifacts and verifies target coverage, source revision, version,
test results, archive hashes, and package smoke records. It creates a draft
GitHub Release, uploads all assets, then publishes it. Only this job has
`contents: write`; build jobs remain read-only. No extra personal access token
is required: publication uses the job's `GITHUB_TOKEN` through GitHub CLI.

The exact `v<workspace-version>` tag is a normal release; validation, RC, and other
tags are prereleases and do not become Latest. Version-shaped tags must match
the workspace's base version. Already published releases are not overwritten;
use a new tag. Failed uploads leave a draft that a rerun can resume. Rerunning a native job
replaces only its temporary Actions artifact, so rerunning a failed publisher
can reuse the completed build artifacts from the same run.

The release has exactly six uploaded assets: four platform archives, each with
SQLite included, `SHA256SUMS`, and `validation-evidence.zip`. The checksum file
covers all four packages and the evidence ZIP. Individual checksums, native
validation records, package smoke records, logs, and `release-manifest.json` are
bundled inside that one evidence ZIP. These release assets are
independent of the seven-day Actions artifact expiration. Windows downloads
contain `scripta.exe` and legacy `scr.exe`; no Rust installation is required.

The publisher writes generated checksums, the release manifest, and release notes
as UTF-8 bytes with LF endings on every host. Native validation logs and smoke
records are copied byte-for-byte. The publication tests emulate Windows text-mode
newline conversion to catch CRLF changes inside the evidence ZIP and checksum
files without relaxing byte comparisons.

The workflow applies to future tag pushes containing this configuration. Existing
tags/releases are not changed retroactively. Review results before updating
platform compatibility claims; queuing a workflow does not establish success.


## Manual promotion of validated releases

[Promote validated release](../.github/workflows/promote-release.yml) is a
separate `workflow_dispatch` workflow with no push or pull-request trigger.
Install it on the default branch, as required by
[GitHub's manual dispatch rules](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#workflow_dispatch).
Its Python tests and verifier run without compiling Rust. It reads published
candidate assets, verifies the successful native run and unchanged package
hashes, then creates a stable tag at the validated commit using `GITHUB_TOKEN`.
That token's tag event does not start another native workflow. The existing
tag-triggered workflow remains unchanged.

See [promotion instructions](releases.md#promote-validated-packages-without-rebuilding)
for inputs, read-only preparation, provenance, and retry rules. Promotion tools
may be newer than the validated source; the workspace version and stable tag
always come from the validated commit. Generated metadata uses LF bytes while
native evidence is preserved verbatim. Run the Python suite after changing
either publication path:

```sh
python3 -m unittest discover -s scripts -p 'test_*.py'
```

## Formatter and diagnostic tooling

`scripta fmt [--check] <file>` and `scripta fmt <file> --check` share the public
`scr_cli::formatter::format(source)` implementation. Formatting uses four spaces
per indentation level, puts each statement on its own line, normalizes operator/
comma spacing, and indents unbraced bodies without adding braces. Original token
spellings (including every string literal), comments, BOM/shebang, `<?scripta`, and
optional `?>` are preserved. Whitespace outside literals/comments uses LF and one
final newline; existing blank-line runs are normalized. Interior multiline comment
and string bytes are retained exactly. Unsupported/malformed scripts are rejected
without any write. The formatted output is re-lexed to verify identical token kinds
and values before it can be saved. The file writer atomically replaces the
canonical target, preserving permissions and a symlink used to address that target.
`--check` never writes; a difference produces E0502 and exit 1.

Git attributes keep `.scripta`, `.scta`, and legacy `.scr` source files at LF
on every platform, including Windows checkouts with `core.autocrlf=true`.
Formatter snapshots and benchmark fixtures override text conversion to preserve
their exact mixed newline bytes. The native validation-driver tests exercise
these checkout rules in an isolated Git repository.

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

A local macOS arm64 comparison with Rust 1.99.1 produced these median
milliseconds at 2048 input entries using the optimized Cargo bench profile.
The before and after runs used the same harness. Results are machine-dependent,
not a cross-platform guarantee; small-input timings are also sensitive to
noise and hash-index overhead.

| Operation | Before indexes | With indexes | Speedup |
| --- | ---: | ---: | ---: |
| Merge, disjoint keys | 10.006 | 0.683 | 14.6× |
| Replace, disjoint keys | 5.496 | 0.401 | 13.7× |
| Combine, distinct keys | 6.000 | 0.467 | 12.9× |
| Flip, distinct values | 4.934 | 0.461 | 10.7× |
| Indexed column, distinct indexes | 5.137 | 0.534 | 9.6× |
| Key difference, disjoint keys | 5.346 | 0.375 | 14.3× |
| Key intersection, identical keys | 2.903 | 0.368 | 7.9× |
| Value difference, disjoint values | 8.165 | 8.134 | 1.0× |


## Distribution validation

Release profiles, the four native build targets, reproducible archive metadata,
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

## Elapsed-time measurement

`Clock::monotonicMs` and `HttpResponse.durationMs` use `std::time::Instant` in
`scr-stdlib`; no calendar timestamps or new runtime backends are involved. The
clock uses one process-local origin and returns fractional milliseconds. HTTP
duration is frozen at bounded-body EOF, including redirects and body buffering.
Tests cover cross-runtime clock ordering, scalar budgets, cancellation, delayed
redirects plus streamed bodies, error statuses, and externally timed failures.
See the [public timing contracts](standard-library.md#monotonic-timing).

Web hosting changes span `scr-web`, request-scoped `scr-stdlib::web`, CLI wiring,
and the cached-program runtime interface. Focused checks are:

```sh
cargo test -p scr-stdlib --test web --locked --offline
cargo test -p scr-runtime --test cached_program --locked --offline
cargo test -p scr-web --lib proxy --locked --offline
cargo test -p scr-cli --test serve_config --locked --offline
cargo test -p scr-cli --test web_server --all-features --locked --offline
cargo test -p scr-cli --test web_server --no-default-features --locked --offline
```

The host uses Hyper for HTTP framing and Tokio for bounded asynchronous network
and worker-process I/O, rather than a custom HTTP parser. New dependencies remain
compatible with Rust 1.85. The subprocess fixtures bind localhost and require
permission to spawn persistent workers. Configuration tests cover CLI/file
precedence, read-only inspection,
private storage paths, safe errors, and timeout boundaries. Web tests execute the
shipped `examples/serve.toml` configuration and exercise connection, header, and
shutdown deadlines. See [server configuration](web-servers.md#server-configuration)
for inspection commands and path resolution. The web example must be executed
through
`scripta serve`, including HTTP requests; an ordinary CLI run has no Web context.

Cookie/session changes additionally use `scr-stdlib --test web_sessions` and the
session scenarios in `scr-cli --test web_server`: cross-worker locking, rotation,
revocation, quota/expiry, persistent restart, and rollback/lock release after a
worker deadline. Cookie parsing/encoding, attributes/prefixes, byte boundaries,
private storage and cancellation are covered by native integration tests.
`getrandom` supplies OS-backed session IDs; `serde` derives a private tagged
storage format. Both were already locked dependencies; no custom random generator
or credential serialization via debug strings is used.

HTML escaping has focused integration coverage in `scr-stdlib --test html` for
strict signatures, Unicode/entities, failed assignment, exact expanded-output
budgets, and cancellation. A direct native unit test also checks cancellation
without relying on evaluator checkpoints. The `html-escaping.scripta` example is
executed with markup/quote-containing input by the CLI release-example target.

Password hashing uses RustCrypto `argon2` 0.5.3 (declared MSRV 1.65), its PHC parser,
and `zeroize` workspace cleanup. The locked `base64ct` version declares Rust 1.85,
matching the workspace minimum. Argon2 implements the primitive; no local password
hash or comparison algorithm is introduced. Host ceilings cover encoded costs,
input lengths, memory and combined work, separate from logical value budgets.

Focused checks: `cargo test -p scr-stdlib --test password` and
`cargo test -p scr-stdlib --lib password`, with the native target/locked/offline
flags used elsewhere. PHP-generated public fixtures live in
`tests/fixtures/passwords/`; Cargo tests do not require PHP. The CLI example checks
actual hashing/verification. Credential-redaction regression tests also cover
qualified Password:: function names alongside real credential fields.

Multipart uploads use Multer 3.1 (without JSON/default features) and MIME 0.3 in
`scr-web`; Tokio's filesystem feature supplies bounded disk reads. Keeping the
parser in the host avoids HTTP backends in the runtime and avoids maintaining a
custom multipart grammar. Source review of Multer identified unbounded raw-header
buffering, so the host wraps its reader with a header-intake budget and yields
between 8 KiB chunks; normalized header limits are checked separately.
Focused regressions are `cargo test -p scr-stdlib --test web_uploads`,
`cargo test -p scr-stdlib --lib web::uploads`, `cargo test -p scr-web`, and
`cargo test -p scr-cli --test web_server`. These cover private staging, disk quota
recovery, exact byte budgets, cancelled copies, untrusted filenames, multipart
validation, supervised timeouts, and the runnable browser upload example. Local
HTTP/process tests need an environment that permits localhost sockets and child
processes. New parser dependencies must continue to compile on Rust 1.85.


## Output buffering and incremental web delivery

Generic application buffers live in `scr-runtime::output`; the eleven public
native helpers are registered by `scr-stdlib`. Pending-buffer bytes, nesting,
and cumulative generated-byte limits are independent of individual value limits.
The web worker seals sessions and commits metadata before its first transport
flush. IPC body chunks are at most 64 KiB and use a one-slot response channel;
terminal failure is tracked separately so a full channel cannot hide an error.
Client body drop signals cancellation, and the request deadline covers blocked
pipe/channel operations. No streaming backend or event loop lives in the core.

Focused checks are `cargo test -p scr-runtime --lib output`,
`cargo test -p scr-stdlib --test output`, `cargo test -p scr-web`, and
`cargo test -p scr-cli --test web_server`. HTTP tests synchronize incremental
responses using explicit client-controlled file barriers, rather than assuming
that a sleep proves early delivery. They cover nested buffers, session sealing,
late metadata rejection, binary bodies, empty/HEAD responses, output ceilings,
slow readers, post-commit failures, and upload cleanup on client disconnect.
The shipped `examples/web-streaming.scripta` is executed through real HTTP.


## Binary streams

`cargo test -p scr-stdlib --test binary_streams` exercises binary file modes,
exclusive creation, capability/root/symlink/read-only checks, ownership/spoofing,
exact budgets and EOF probes, handle quotas, copy validation, incremental HTTP
responses, streamed request bodies, redirects, TLS rejection, TCP partial I/O,
UDP empty/truncated packets, deadlines, and cancellation. Preserve the separate
`--test streams` line-reader suite and `--test json_http_modules` client suite.
Fixtures use temporary directories and localhost sockets, never public endpoints.
The new implementation needs no additional dependencies and retains Rust 1.85.
`release_examples` executes binary copying and HTTP streaming uploads through the
CLI with controlled local input and verifies bytes/results. New outgoing stream
capabilities are `network.connect` and `network.datagram`; hosts can remove them.
Low-level bind/listen/accept, readiness, and socket options await the next milestone.


## Socket and transfer validation

`cargo test -p scr-stdlib --test sockets` covers listener/accept, nonblocking
would-block versus EOF, advisory readiness, list order/deduplication, half-close,
bound UDP replies/truncation, quotas, refused connections, address conflicts,
foreign/spoofed handles, options, cancellation, and IPv6 loopback where available.
Re-run `binary_streams` when changing the shared readiness implementation.
Mio and Socket2 were already transitive dependencies; direct dependencies expose
safe OS readiness and bind/listen/options APIs, with minimum Rust versions below
1.85. Keep the no-unsafe workspace policy and explicit native-target validation.
OS-specific option results remain platform dependent; no non-native check proves
Windows/Linux socket correctness.

Transfer tests include raw-body spooling, independent cursors, quota sharing,
request expiry, metadata/safe attachment encoding, known-size output preflight,
HEAD suppression, rejected-upload nonexecution, and content inspection without
filesystem-read permission. `web_server` runs the complete raw-upload/download
example over HTTP; `release_examples` runs the socket echo example with binary
input and bounded local subprocess cleanup. No fixtures use public network
services. PHP comparison links document deliberate API differences rather than
claiming global PHP syntax/API compatibility.

Proxy-hosting regressions cover CIDR families/boundaries, bounded forwarded chains,
spoofed/duplicate/malformed headers, authority normalization, effective-host
allowlists, worker IPC metadata, redirects, unchanged cookie security, and IPv6
loopback where available. The shipped proxy script/configuration runs through a
controlled sanitizing local forwarder. This fixture asserts the backend contract;
it does not establish Caddy/NGINX version behavior or public TLS deployment.

Observability coverage includes `cargo test -p scr-web --lib --locked --offline`
for ID exhaustion, saturating counters, readiness/RAII, body outcomes, and bounded
UTF-8 application-log frames. The CLI configuration/web suites additionally test
JSON correlation/privacy, caller-ID rejection, buffered/streamed errors, client
disconnects, deduplicated timeouts, health route separation, overload/recovery,
and management readiness during a real termination drain. Execute the
`examples/observe-serve.toml` configuration through controlled application and
management HTTP requests. Host logs intentionally omit source/error text; use
`scripta check` for local syntax diagnostics and keep explicit application logs
free of secrets. Management probes never execute application scripts.

Shutdown regressions in `scr-cli --test web_server` cover idle and
partial-header connections, active keep-alive/pipelined requests, queued work,
repeated Unix termination signals, forced stream truncation, worker reaping,
request spool cleanup, saved sessions, and a controlled two-instance front-end
switch. Signal tests are Unix-gated; they do not establish Windows console/service
behavior. Deployment/service templates and an operator command sequence live in
[web hosting](web-servers.md#deployment-and-service-management) and
[`examples/deploy`](../examples/deploy/). The front-end fixture validates backend
replacement behavior; it does not run Caddy or install a service manager job.

Worker resilience coverage uses `scr-web --lib` for backoff cap/reset/overflow,
unexpected worker-loop panic recovery, and synthetic malformed/truncated/out-of-order
IPC through local child pipes. The CLI configuration suite checks default/disabled,
exact maximum, out-of-range, type, and precedence behavior for
`worker-max-requests`. The HTTP suite checks count-based recycling and cached
source, stream/spool and session preservation, all idle children lost without an
application request, repeated startup failures, growing retry delays, restored
capacity/readiness, reset after successful service, and shutdown during backoff.
Failure fixtures use private local storage and restore safe permissions before
recovery; signal/Unix-permission cases are Unix-gated. No public network services
or production-only fault-injection knobs are used.

For interactive server development, run the isolated
[`examples/watch`](../examples/watch/) application as documented in
[development watch mode](web-servers.md#development-watch-mode). `scr-web --lib`
regressions cover polling selection/exclusions, symlinks, cancellation, exact/
over-limit entry and byte budgets, depth, invalid snapshots, and RAII cleanup.
The CLI suites cover opt-in/override/bounds, masked local startup diagnostics,
entry/include edits, coalesced rapid saves, failed syntax/startup retaining the
old generation, stable listener reuse, old-stream include isolation, readiness,
spool/snapshot cleanup, ordinary serving without polling, and shutdown with staged
children. The shipped watch script/configuration executes a controlled include
edit over real local HTTP, with source syntax/format checks. Unix-specific signal
and symlink cases remain platform-gated.

## HTTP measurement and native validation

The standalone-server workload and bounds are defined before measurement in
[`benchmarks/serve.md`](../benchmarks/serve.md). The standard-library Python driver
validates response contents/statuses, separates successful throughput and latency
from rejection speed, records resident memory and fixed server gauges, and checks
recovery/spool/child cleanup. It uses private local fixtures and never public HTTP.
Its short functional run is separate from the optimized measurement:

```sh
MACOSX_DEPLOYMENT_TARGET=15.0 cargo build -p scr-cli --profile dist --locked --offline --target aarch64-apple-darwin
python3 -m unittest discover -s benchmarks -p 'test_*.py'
python3 benchmarks/serve.py --scripta target/aarch64-apple-darwin/dist/scripta --seconds 1 --soak-seconds 10 --output /private/tmp/scripta-http-functional-new.json
python3 benchmarks/serve.py --scripta target/aarch64-apple-darwin/dist/scripta --output /private/tmp/scripta-http-measured-new.json
```

Use new output paths; do not overwrite retained evidence. Keep builds/tests idle
during measurement. The default maximum soak window is 60 seconds, with a separate
100000-request ceiling; inspect actual duration and the cap flag. The retained
macOS observation ended at the request ceiling after 55.061 seconds. Results,
hashes, counts, RSS caveats, and the [JSON receipt](../benchmarks/evidence/serve-macos-arm64-20261115.json)
are summarized in [HTTP performance](performance.md#standalone-http-hosting--2026-11-15).
Performance is observational; correctness and cleanup determine driver success.

For this milestone, native local validation used macOS 27.0.1 ARM64, pinned Rust
1.99.1 compiler host/target `aarch64-apple-darwin`, explicit targets and deployment
setting 15.0. Run the following separately from measurements:

```sh
cargo fmt --all -- --check
MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p scr-stdlib --lib collections --locked --offline --target aarch64-apple-darwin
MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p scr-stdlib --test collections --locked --offline --target aarch64-apple-darwin
MACOSX_DEPLOYMENT_TARGET=15.0 cargo test --workspace --all-features --locked --offline --target aarch64-apple-darwin
MACOSX_DEPLOYMENT_TARGET=15.0 cargo test --workspace --locked --offline --target aarch64-apple-darwin
MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p scr-cli --test web_server --test serve_config --test sqlite_extension --no-default-features --locked --offline --target aarch64-apple-darwin
MACOSX_DEPLOYMENT_TARGET=15.0 cargo clippy --workspace --all-targets --all-features --locked --offline --target aarch64-apple-darwin -- -D warnings
MACOSX_DEPLOYMENT_TARGET=15.0 cargo +1.85.0 check --workspace --all-targets --all-features --locked --offline --target aarch64-apple-darwin
```

Validation on 2026-11-15 (America/New_York), against base revision
`df6607e38bda822248e9b53396aef1a0fdfd5fc1` plus the recorded measurement changes,
passed 639 tests each with default and all features, 80 minimal-feature tests,
six collection unit tests, and 21 collection integration tests. Formatting,
all-target/all-feature Clippy with warnings denied, and the Rust 1.85.0
all-target/all-feature workspace compile check passed. The benchmark Python
suite passed seven tests; the validation/publication Python suite passed 11.
The benchmark fixture passed syntax/format checks and real local HTTP execution.
The minimum-Rust command checks compilation, not runtime tests. The benchmark
requires SQLite; minimal-feature behavior is covered by separate native tests.
No Linux/Windows/native macOS 15 host was available for this milestone. Retain
those validations as pending and preserve historical compatibility receipts.
A deployment target alone is not execution on macOS 15. Full native release
support additionally requires the dist tests, inspection, packaging, and extracted
smoke steps in [releases](releases.md); this work publishes no release assets.
