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
wall-time thresholds in CI; correctness and cleanup regressions are tested there.
