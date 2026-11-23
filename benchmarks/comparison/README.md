# PHP, Python, and Scripta CLI comparison

This suite compares ten fixed workloads, not an overall language ranking. Each
fixture prints a deterministic result checked on every invocation, including
warmups. Child output accepts LF or CRLF line endings. There are no extra Python
packages or public network dependencies.
The original Bash/Python/Scripta suite in `../run.py` remains independent.

## Reproduce

From the repository root, build an optimized native Scripta with the pinned Rust
toolchain. Confirm the host from `rustc -vV` matches the explicit target. On Apple
Silicon:

```sh
MACOSX_DEPLOYMENT_TARGET=15.0 cargo build -p scr-cli --release --locked --offline --target aarch64-apple-darwin
python3 benchmarks/comparison/run.py --scripta target/aarch64-apple-darwin/release/scripta --output benchmarks/comparison/evidence/local.json
python3 benchmarks/comparison/render.py benchmarks/comparison/evidence/local.json --readme README.md --performance docs/performance.md
```

On another platform, use its native target and executable path (`scripta.exe` on
Windows), and omit `MACOSX_DEPLOYMENT_TARGET`. Omit `--offline` if dependencies
are not cached. PHP and Python must also be native executables on the same host.
Use `--php PATH` and `--python PATH` to choose them explicitly. Choose a new
output filename for every run; the driver never replaces existing evidence.
`--output` is optional: omit it to print timing rows without saving a file.
Add `--print` to print the full JSON report to stdout, with timing progress on
stderr. Combine `--print` and `--output PATH` to both display and save the report.

```sh
python3 benchmarks/comparison/run.py --scripta target/aarch64-apple-darwin/release/scripta
python3 benchmarks/comparison/run.py --scripta target/aarch64-apple-darwin/release/scripta --print
```

All JSON files under `benchmarks/` are ignored by Git. Keep raw measurement and
profiling reports locally; only rendered Markdown summaries belong in the repository.

The default is 3 warmups followed by 15 measured samples per workload/runtime.
Languages run sequentially in a seeded, shuffled order for each round. There is
no parallel timing. Close other busy applications and finish builds/tests before
measurement. `--samples`, `--warmups`, `--seed`, and `--timeout` are configurable;
the default per-invocation timeout is 60 seconds. The README renderer accepts only
a complete matrix with consistent medians and finite, positive raw observations.
Missing runtimes produce a partial report and a nonzero exit; invalid outputs,
timeouts, and child failures stop measurement. Failed evidence is saved when
`--output` is supplied and printed when `--print` is supplied.

## What is measured

Times use Python's monotonic performance counter around a fresh subprocess,
including launch, parsing, runtime initialization, work, in-script correctness
checks, and output. Parent-side output validation is outside the timed region.
The filesystem cache is warm; processes and runtime caches start fresh for every
sample. Warmups do not create a long-lived interpreter. Startup is reported
separately and never subtracted. Raw JSON contains every observation plus min,
median, and max; the README and performance reference show the same medians in
milliseconds without speed ratios. The README uses a generic architecture label; exact machine
and OS details appear only in the raw evidence.

PHP runs with `-n` (no php.ini), `opcache.enable_cli=0`, `opcache.jit=0`, and
`pcre.jit=0`. Python runs with `-I` (isolated mode). The harness sets `LC_ALL=C`,
`PYTHONHASHSEED=0`, and `PYTHONNOUSERSITE=1`, and removes inherited `SCR_*`
settings so local permissions or budgets cannot silently alter a comparison.
Python isolated mode ignores `PYTHONHASHSEED`; dictionary hash randomization
therefore remains enabled. These are CLI configurations, not server benchmarks
or JIT-enabled performance claims.

Evidence retains runtime versions and binary hashes, compiler, source revision,
working-tree status, fixture/driver hashes, CPU, OS, architecture, configuration,
round ordering, and all samples. Exact runtime versions are kept out of the
published tables. A recorded dirty tree identifies local benchmark additions; the
fixture hashes identify the actual measured scripts. This suite does not prove
release support or validate other operating systems.

## Workload contracts

| Workload | Contract |
| --- | --- |
| Numeric loop | 1,000,000 iterations, accumulating `(i % 97) * 3` using integers; result fits signed 64-bit arithmetic. |
| Recursive Fibonacci | Naive two-branch recursion, `fib(28)`, without memoization; result 317811. |
| List pipeline | Build 2,000 sequential integers, then 200 materialized map/filter/sum passes. Mapping triples values; filtering retains even values; explicit summation loop in every language. PHP reindexes filtered output to match contiguous lists. |
| String concatenation | 100,000 explicit `text = text + chunk` operations (PHP/Scripta use `.`), each appending eight ASCII bytes; verify length and complete removal of the repeated chunk. No join/builder substitution. Runtime concatenation optimizations are permitted. |
| Dictionary operations | Insert 256 prefixed string keys, perform 40,960 read/update/read iterations, then 128 removals producing a new dictionary each time. Python uses `copy`, PHP assignment uses copy-on-write, Scripta uses `Map::remove`. This includes collection copy costs rather than isolating hash lookup. Verify count, read checksum, and remaining value sum. |
| Class dispatch | Construct one counter, invoke its mutating method 200,000 times, verify final field and accumulated return values. Includes loop and field access costs; construction happens once. |
| Regex match | 20,000 calls with a fixed anchored ASCII pattern and four fixed inputs (half match). Use the public matching API each time; includes each runtime's native compilation/caching behavior, not precompiled engine-only throughput. PHP PCRE JIT is explicitly disabled. |
| JSON roundtrip | 20,000 compact encodes and decodes of the same nested object with a list, strings, integers, and a boolean. Check full decoded equality each iteration and accumulate decoded fields. JSON text/key order need not match across runtimes. |
| Function calls | 200,000 calls to an increment function and accumulation of returned integers. |
| Startup | Empty source; PHP requires its opening tag. |

Algorithms and work sizes are shared; native APIs and language semantics still
differ. Scripta's strict type checks and owned collections, PHP's array model,
and Python's integer/object model are part of these results. Do not infer
application performance or a universal winner from this selection. Startup can
still matter in the shorter rows. Any changes to workloads or runtime settings
require a fresh complete run before updating the tables.

## Validation

```sh
python3 -B -m unittest discover -s benchmarks/comparison -p 'test_*.py'
```

Tests cover real child output and exit validation, timeout termination, explicit
missing executables, failed/partial evidence, complete table generation, corrupt
samples, and explicit LF/CRLF document byte fixtures. Run Scripta `check` and
`fmt --check` on each `.scripta` fixture; a full comparison run executes and
validates all three languages. Keep benchmark timing separate from tests.

## Execution-only profiling

The separate worker measures lexing, parsing, runtime/library setup, parsed-program
execution, and runtime teardown. It uses the unchanged Scripta comparison fixtures
and checks both output and normal completion on every execution. Each sample gets
fresh runtime globals. Lex/parse result destruction is outside those timers;
execution includes the runtime's entry/exit collections and output buffering.
Setup and runtime destruction are timed separately. Parsing happens once for the
program used in execution samples. The public parsed-program API has no attached
source text or CLI module resolver here, so its execution timings are not an exact
subtraction of CLI startup from whole-process timings. Do not compare individual
small differences across separate runs as an isolated component's cost.

For the temporary optimization sequence, preserve the original CLI executable
and its hash before rebuilding. The baseline manifest must identify that binary,
retained CLI evidence, compiler, source revision, and fixture hashes. Store local
binaries under ignored `target/performance-baselines/`; do not commit binaries.
The current baseline manifest is `target/performance-baselines/step1/manifest.json`.
Build the symbolized optimized worker on the native host, then measure it without
concurrent builds, tests, or profiler instrumentation. For Apple Silicon:

```sh
MACOSX_DEPLOYMENT_TARGET=15.0 CARGO_PROFILE_RELEASE_DEBUG=1 cargo build -p scr-cli --example comparison_profile --release --locked --offline --target aarch64-apple-darwin
python3 benchmarks/comparison/execution.py --worker target/aarch64-apple-darwin/release/examples/comparison_profile --baseline-manifest target/performance-baselines/step1/manifest.json --output benchmarks/comparison/evidence/execution-local.json
```

Use the corresponding native target and `.exe` suffix on Windows. The worker
rejects debug builds. This recipe enables debug information while retaining
release optimization for native profiling; record and use the same recipe for
future baseline/candidate comparisons. Execution reports preserve all five stages'
raw observations and the worker/source/driver hashes. They do not update the
published PHP/Python/Scripta tables.

Native profiling is a separate, macOS-only optional driver:

```sh
python3 benchmarks/comparison/native_profile.py --worker target/performance-baselines/step1/comparison_profile --kind cpu --directory target/performance-baselines/step1/cpu-local
python3 benchmarks/comparison/native_profile.py --worker target/performance-baselines/step1/comparison_profile --kind alloc --directory target/performance-baselines/step1/alloc-local
```

CPU capture uses `sample` during repeated checked executions with fresh runtimes;
its stacks also include setup and teardown, unlike the execution-only timer.
Allocation capture uses `MallocStackLogging=full` and `malloc_history` on a single
checked execution held alive for inspection. It reports allocation stacks live
at the allocator's recorded high-water mark and after execution, plus recorded
lifetime allocation counts and cumulative heap/VM sizes from the log. These totals
include startup, parsing, setup, execution, and logging overhead; compacted logs
can omit immediate allocation/free pairs. They are not exact Rust allocation
counts or a hard RSS ceiling. Peak stacks can include profiler startup allocations. Logging changes performance, so never
use instrumented wall times as benchmark results. Profiler output, subprocess
failures, worker identity, and hashes are saved in a new local capture directory.
Native process inspection may require permission outside a sandbox. Other hosts
need their own native profilers; this driver does not establish cross-platform
profiling or performance results.
