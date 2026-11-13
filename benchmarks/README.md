# Repeatable interpreter benchmarks

Build and run from the repository root:

```sh
cargo build --release --locked -p scr-cli
python3 benchmarks/run.py --scr target/release/scr --output benchmarks/results/local.json
cargo bench -p scr-cli --bench interpreter --locked
```

The Python standard-library runner uses two warmups and 11 samples by default.
Override with `--warmups N --samples N`. It creates its own temporary input files,
validates every fixture's output, records wall time and per-child peak RSS when
`os.wait4` exists, and writes JSON. It invokes programs with argument arrays and
never asks a shell to interpret an assembled command. Bash fixtures are explicitly
run with Bash because Bash is the subject of that comparison.

To compare two locally preserved release builds in one measurement run:

```sh
python3 benchmarks/run.py --scr target/release/scr --reference /path/to/before-scr --output benchmarks/results/paired.json
```

The runner records executable sizes/hashes and fixture hashes. Output filenames
are explicit; choose a new filename to retain earlier runs. This is warm-cache
launch latency, not a cold-start test. Avoid running tests/builds or other CPU
work concurrently with measurement. The harness reports actual observations;
missing Bash, `true`, native map support, or per-child RSS appear as unavailable.
Bash 3.2 cannot run the associative-array fixture. The nested-copy memory fixture
has no Bash counterpart. No public network services are used.

Fixtures cover empty startup, scalar access, calls, nested loops, concatenation,
lists, maps, whole-file reads, direct child processes, and retained collection
copies. Work counts are constants in fixtures and expected checksums are in
run.py. The file input is 1 MiB of ASCII x bytes; memory input is 4,096 integers.
Bash uses a global function result instead of a subshell per call and uses cat
for whole-file reads. Python explicitly deep-copies in the memory fixture to match
Scripta's observable copy semantics. These are equivalent output workloads using
each language's idioms, not a universal language performance ranking.

The Rust `interpreter` harness emits CSV:
`fixture,phase,observations,median_ms`. Lex/parse use 1,000 observations; execution
uses nine fresh runtimes with setup outside the timer. Sharing experiment rows
instead report one 10,000-operation batch total for owned clone, Rc clone, and
Rc::make_mut after snapshot. Capture stdout to a new CSV if retaining a local run.
Baseline rows predate the representation experiment; final rows include it.

Actual Phase 13 measurements, platform details, caveats, and the safety audit are
in [docs/performance.md](../docs/performance.md). Performance is not gated on noisy
wall-time thresholds; correctness and cleanup regressions have deterministic tests.

## Long-running memory behavior

Deterministic lifecycle checks run with the normal Rust tests:

```sh
cargo test -p scr-runtime --locked
```

They cover 10,000 transient operations in a reused runtime, scope arena slot
reuse, repeated exceptions and instruction-limit failures, discarded closure
reclamation, host-held callbacks, recursive cycles, scope-limit enforcement, and
resource release when runtimes drop.
Resource drop counters verify ownership cleanup; they do not measure allocator
bytes or claim that every allocation is tracked.

Build the isolated soak worker, then collect current RSS samples:

```sh
cargo build -p scr-runtime --example memory_soak --release --locked
python3 benchmarks/memory_soak.py --output benchmarks/results/memory-soak-local.json
```

The driver runs four separate processes: temporary collections, caught errors,
discarded closures, and repeated runtime creation/destruction. Each runs three
warmup batches followed by 20 measured batches of 500 iterations by default.
Use `--warmups`, `--batches`, and `--iterations` to extend a run. Repeat
`--workload transient --workload errors --workload recreate` to select a subset.
All four workloads can be included in extended runs. `--worker` accepts
an explicit executable path for target-specific builds. JSON output paths must
be new files, and their parent directories must exist. Reports include the worker
SHA-256 for identifying the measured build. The driver fails on worker
errors, invalid checkpoints, and checkpoint timeouts (`--timeout`, seconds).

At every checkpoint the worker waits while the driver samples current resident
memory using `/proc/PID/status` on Linux or `ps` on macOS. This avoids confusing
peak RSS with current memory. Other platforms report null RSS while still
validating workload completion. The worker parses each batch program once and
discards script output; the report's growing sample list lives in the driver,
not the measured child. The final sample is taken after the runtime is dropped
but before the worker exits.

All four workloads should settle after warmup. Discarded closure environments
are now traced and reclaimed automatically, including during a single long
execution. Deterministic tests check cleanup within that execution as well as
between batches.
Freed allocations may remain resident in the allocator, so a runtime drop need
not return RSS to the startup value. No RSS thresholds are enforced yet: establish
repeatable native baselines before choosing platform-specific growth bounds.
Compare multiple batch samples and repeated runs rather than a single delta.
The recreate workload drops the previous runtime before constructing the next;
its batch-boundary samples include one live runtime.

## Tracing collector performance

Run the deterministic collector checks in both debug and optimized builds:

```sh
cargo test -p scr-runtime --locked
cargo test -p scr-runtime --profile dist --locked
python3 -B -m unittest discover -s benchmarks -p test_collector.py
```

Test-only work counters check linear graph traversal, deduplication of 10,000
aliases to shared captures, bounded arena/weak-registry storage over 20,000
recursive-closure operations, and adaptive collection scheduling. Ordinary loops
must not trigger a collection at every checkpoint. Counters are absent from
production builds; these tests gate correctness and operation counts, not time.

Build the optimized benchmark workers and run the complete matrix:

```sh
cargo build -p scr-runtime --examples --release --locked
python3 benchmarks/collector.py --output benchmarks/results/collector-local.json
```

The matrix uses 1,000, 10,000, and 100,000 items, two warmup trials, and 11 measured
trials per case. Cases cover discarded captures, fully live captures, 90% live
captures, discarded recursive cycles, shared captures, nested collections,
host-held callbacks, plain scalar collections, ordinary script execution, and
closure churn. Each pause trial constructs a fresh graph outside its timed
section. It times one explicit collection, checks the exact reclaimed scope
count, checks surviving callback results, and confirms a second collection
reclaims nothing. After live roots are removed, their captures must also become
reclaimable. This avoids accidentally timing repeated collection of an empty
fixture. The size counts factory calls (two captured scopes each), except for
plain values and the two execution workloads, where it counts items/iterations.

Reports include raw nanosecond observations, median, nearest-rank p95, maximum,
per-item median, reclaimed scope counts, platform, and worker SHA-256. With fewer
than 20 samples, nearest-rank p95 is the maximum; increase `--samples` when studying
tail latency. Execution rows include automatic collection and execution-boundary
collection; they measure total script cost, not isolated GC overhead.

Use `--case` repeatedly to select cases, `--sizes`, `--samples`, and `--warmups`
to change the matrix, and `--reference /path/to/preserved/collector_bench` to
compare compatible worker builds. Reference/candidate ordering alternates across
rows; ratios greater than one mean the candidate's median was slower. Workers
with debug assertions enabled are rejected. The output file must be new.
Run on an otherwise idle machine without concurrent builds or tests.

After timing trials finish, the driver runs the existing four-process RSS soak
separately and includes its samples in the JSON report. Increase
`--memory-batches 100 --memory-iterations 2000` for a longer run, or use
`--skip-memory` for timing only. `--soak-worker` accepts a target-specific memory
worker path. RSS is not sampled during timing trials. No elapsed-time or RSS
thresholds are enforced: establish repeated native baselines before selecting limits.
The timings cannot establish a hard pause guarantee, and the ordinary workload
cannot establish GC-on versus GC-off overhead without a suitable reference build.
