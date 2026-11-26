# Scripta, Node, PHP and Python callback comparison

This suite compares eager callback workloads with complete result checks. It
measures execution in warmed, persistent runtimes separately from CLI startup.
It does not measure an overall language ranking or prove a cause for a timing gap.
No external packages, downloads or network fixtures are required.

## Reproduce

Build with the pinned compiler on the native target. On Apple Silicon:

```sh
MACOSX_DEPLOYMENT_TARGET=15.0 cargo bench -p scr-stdlib --bench callback_comparison --no-run --locked --offline --target aarch64-apple-darwin
MACOSX_DEPLOYMENT_TARGET=15.0 cargo build -p scr-cli --release --bin scripta --locked --offline --target aarch64-apple-darwin
```

Use the benchmark executable path printed by Cargo (its filename includes a build
hash). Replace ENGINE, NODE, PHP and PYTHON with native executable paths:

```sh
python3 benchmarks/callbacks/run.py --engine ENGINE --scripta target/aarch64-apple-darwin/release/scripta --node NODE --php PHP --python PYTHON --output benchmarks/callbacks/evidence/local-run
```

Use the appropriate native triple/executable suffix on other platforms; omit the
macOS deployment setting elsewhere. Close busy applications and finish builds and
tests before measurement. The driver runs engines sequentially, never in parallel.
Choose a new output directory each time; existing evidence is never overwritten.
JSON evidence is locally retained and ignored by Git. `--rounds` defaults to 4
(`--pairs` remains an alias); `--sizes` defaults to 128 2048 8192, with a maximum
of 8192 to bound the quadratic case. Explicitly choose the interpreter versions;
a system Python can be considerably older than other installed versions.

## Workloads and semantic differences

| Case | Scripta | Node | PHP | Python |
| --- | --- | --- | --- | --- |
| List map | List::map | Array.map | array_map | list(map(...)) |
| Captured map | Captured closure | Captured closure | Captured closure | Captured closure |
| List filter | List::filter | Array.filter | array_values(array_filter(...)) | list(filter(...)) |
| List reduce | List::reduce | Array.reduce | array_reduce | functools.reduce |
| Map transforms | Typed-key ordered Map helpers | JS Map loop adapters | Ordered-pair loop adapters | Ordered-pair comprehension/loop adapters |
| Record map | Rank callback | Rank callback | Rank callback | Rank callback |
| Key sort | List::sortBy | Decorate/sort/undecorate | Decorate/usort/undecorate | sorted(key=...) |
| Comparator sort | List::sortWith | Copy array, then sort | Copy array, then usort | sorted(key=cmp_to_key(...)) |
| Growing-copy reduce | Value-semantics append | Spread and append | Value-semantics append | List concatenation |
| Additional mutating reduce | — | Node-only push | — | Python-only append |

Inputs are shared JSON fixtures constructed before timing. Values are integers
0..n-1, within JavaScript's exact integer range. Map keys alternate integers and
strings, retaining key type and insertion order. Records have unique ids, rank
`(id * 73) % 97`, and nested ASCII/Unicode string payloads. Repeated, shuffled
ranks exercise stable ties; sorting leaves the input unchanged. An independent
Python reference constructs complete expected outputs. Each measured batch's
last result is checked outside timing, preserving typed keys and exact record
ordering. All four fixture readers have explicit LF/CRLF smoke checks.

PHP arrays coerce numeric string keys, so its Map cases deliberately retain keys
as values in ordered pairs. Python's Map cases use the same ordered-pair adapter
contract; these are not native dictionary performance measurements. JS Map has
no built-in map/filter/reduce methods, so its Map cases also use explicit adapters.
PHP filtering includes reindexing to match List output. Python map/filter are
fully consumed into lists; iterator creation alone is not a matched result.

Python's native key sort computes each key once; its comparator requires
functools.cmp_to_key wrappers. Node/PHP key sorts use benchmark adapters whose
key-decoration/undecoration allocations are included. These are API-level
alternatives, not identical algorithms. Node/Python share references to records,
PHP uses copy-on-write arrays, and Scripta copies nested collection values during
argument/result handling. No object mutation occurs, so outputs agree without
forcing deep copies on the other engines. Additional Node/Python mutating
reductions have different accumulator semantics from the common copying case.

## Measurement

Each fresh process handles all three sizes. Every case warms for at least 100 ms,
then calibrates a batch toward 10 ms, capped at 10,000 operations. Eleven measured
batches alternate forward/reverse case order. Four fresh process rounds rotate
the runtime order, giving each engine every position once. Tables report the
median of four per-process medians; evidence retains all samples, batch sizes
and the range of process medians. This is a warmed workload comparison; 100 ms
does not guarantee final V8 optimization state. No forced garbage collection,
CPU pinning or timing pass/fail thresholds are used. GC during timed operations
is included; untimed result validation also allocates and can influence later GC.

Scripta uses the optimized Rust release profile with overflow checks, the public
Runtime::new defaults (recursion 64; no configured instruction/value-byte/scope/
output quotas or interrupt flag), and standard-library registration. Depth checks,
checked arithmetic, callable validation, collection copying, normal checkpoints,
and result assignment/execute cleanup remain inside execution timing. Fixture
construction, registration, callback definitions, parsing, and full output checks
are outside. Other engines assign each result to a retained sink; allocations
and callbacks are inside timing.

Node uses default V8/JIT behavior. PHP uses `-n`, `opcache.enable_cli=0` and
`opcache.jit=0`; this is a PHP CLI comparison without OPcache/JIT, not a
JIT-enabled PHP claim. Its explicit 512 MiB memory limit accommodates decoded
reference fixtures; the other engines use their ordinary heap policy. Python
uses `-I` isolated mode. Inherited NODE_OPTIONS/NODE_PATH and SCR_* settings are
removed, and LC_ALL=C. Isolated Python ignores PYTHON* environment settings;
hash randomization remains enabled. Interpreter versions and settings are in
each report; the driver also identifies PHP_BINARY behind a launcher.

This compares embedded Scripta execution with language-level functions, including
different data models, algorithms, boundaries and safety checks. It does not
isolate native invocation overhead or distinguish its cost from callback
evaluation, allocation, copying or GC. Profiling is needed before optimization.

Startup is separate: 3 warmups then 15 samples per engine, rotating runtime order,
using an empty Scripta CLI script, `node -e ''`, `php ... -r ''`, and
`python -I -c ''`. The optimized Scripta CLI includes default SQLite; other flags
match the execution run. Complete subprocess launches are timed; filesystem caches
are warm, each process is fresh. Startup is never subtracted from execution.

Evidence includes revision/dirty-tree state, driver/fixture/binary hashes, compiler,
host architecture, runtime versions, command arguments, environment overrides,
raw timing samples and process order. A local macOS result does not establish
Linux/Windows behavior, macOS 15 validation or dist/package/release support.

## Local results

Run date: 2026-11-26; revision `396a5626486509cb0235716f2846156ddbc58c53` plus benchmark-only
working-tree changes. Host: macOS 27.0.1 ARM64 (26A434); rustc 1.99.1
(58cf8f9c3), native `aarch64-apple-darwin`, release profile, deployment setting
15.0. Node 22.22.2 / V8 12.4.254.21-node.39; PHP 8.4.20 CLI without
OPcache/JIT; CPython 3.13.18 in isolated mode. All interpreters report native
ARM64. All four rounds (16 processes) completed and every output check passed.
Values below are median milliseconds per operation, lower is better.

### 128 elements

| Operation | Scripta ms | Node ms | PHP ms | Python ms |
| --- | ---: | ---: | ---: | ---: |
| List map | 0.039513 | 0.000213 | 0.003303 | 0.004969 |
| Captured-closure map | 0.050226 | 0.000185 | 0.003889 | 0.004183 |
| List filter | 0.046026 | 0.000183 | 0.004675 | 0.004826 |
| List reduce | 0.038973 | 0.000071 | 0.003682 | 0.004254 |
| Map map adapter | 0.057152 | 0.002769 | 0.010084 | 0.007466 |
| Map filter adapter | 0.057124 | 0.001636 | 0.011803 | 0.008834 |
| Map reduce adapter | 0.049411 | 0.000484 | 0.007637 | 0.004313 |
| Record map | 0.099622 | 0.000204 | 0.003936 | 0.005061 |
| Record key sort | 0.151436 | 0.007601 | 0.053164 | 0.007341 |
| Record comparator sort | 0.927297 | 0.006948 | 0.046027 | 0.067486 |
| Growing-copy reduce | 0.174071 | 0.009724 | 0.017226 | 0.022155 |
| Node-only mutating reduce | — | 0.000228 | — | — |
| Python-only mutating reduce | — | — | — | 0.004288 |

### 2,048 elements

| Operation | Scripta ms | Node ms | PHP ms | Python ms |
| --- | ---: | ---: | ---: | ---: |
| List map | 0.240946 | 0.012139 | 0.051679 | 0.076068 |
| Captured-closure map | 0.409225 | 0.012522 | 0.061004 | 0.076723 |
| List filter | 0.342554 | 0.012636 | 0.071711 | 0.074241 |
| List reduce | 0.238423 | 0.009484 | 0.058449 | 0.069393 |
| Map map adapter | 0.518705 | 0.073324 | 0.161941 | 0.146893 |
| Map filter adapter | 0.517486 | 0.035989 | 0.190009 | 0.149965 |
| Map reduce adapter | 0.404324 | 0.007789 | 0.121380 | 0.073327 |
| Record map | 1.241767 | 0.013051 | 0.063999 | 0.074420 |
| Record key sort | 2.103108 | 0.204262 | 1.250070 | 0.139196 |
| Record comparator sort | 24.803083 | 0.186807 | 1.183623 | 1.886208 |
| Growing-copy reduce | 25.846459 | 2.456292 | 2.764135 | 2.597073 |
| Node-only mutating reduce | — | 0.006571 | — | — |
| Python-only mutating reduce | — | — | — | 0.062319 |

### 8,192 elements

| Operation | Scripta ms | Node ms | PHP ms | Python ms |
| --- | ---: | ---: | ---: | ---: |
| List map | 0.872585 | 0.048175 | 0.205115 | 0.296402 |
| Captured-closure map | 1.546220 | 0.049837 | 0.242924 | 0.304620 |
| List filter | 1.270372 | 0.050018 | 0.288926 | 0.297110 |
| List reduce | 0.872889 | 0.037622 | 0.230998 | 0.277012 |
| Map map adapter | 2.027792 | 0.408489 | 0.665031 | 0.768613 |
| Map filter adapter | 1.996829 | 0.161834 | 0.757679 | 0.685030 |
| Map reduce adapter | 1.536628 | 0.030636 | 0.481263 | 0.294499 |
| Record map | 4.920146 | 0.052172 | 0.261097 | 0.289716 |
| Record key sort | 8.494052 | 0.880509 | 6.191198 | 0.599638 |
| Record comparator sort | 122.725584 | 0.810125 | 6.252979 | 8.251188 |
| Growing-copy reduce | 393.712792 | 32.417354 | 42.970250 | 58.727812 |
| Node-only mutating reduce | — | 0.022021 | — | — |
| Python-only mutating reduce | — | — | — | 0.241062 |

### Repeated-process spread at 2,048 elements

Ranges are the minimum/maximum of four process medians, not confidence intervals.
All 11 measured batch observations per process are retained locally.

| Operation | Scripta range ms | Node range ms | PHP range ms | Python range ms |
| --- | ---: | ---: | ---: | ---: |
| List map | 0.240250–0.243509 | 0.012000–0.012378 | 0.051390–0.051824 | 0.075705–0.076368 |
| Captured-closure map | 0.408813–0.414920 | 0.012392–0.012702 | 0.060603–0.061332 | 0.076500–0.076839 |
| List filter | 0.341382–0.344076 | 0.012541–0.013103 | 0.071460–0.072095 | 0.074175–0.074331 |
| List reduce | 0.238068–0.238870 | 0.009431–0.009547 | 0.057777–0.058659 | 0.068915–0.069653 |
| Map map adapter | 0.518012–0.519910 | 0.072069–0.075488 | 0.160229–0.163931 | 0.145443–0.147337 |
| Map filter adapter | 0.516988–0.517985 | 0.035255–0.036434 | 0.187425–0.190569 | 0.149795–0.152055 |
| Map reduce adapter | 0.403035–0.405730 | 0.007585–0.008086 | 0.119719–0.122217 | 0.073196–0.073523 |
| Record map | 1.229167–1.247984 | 0.012903–0.013402 | 0.063621–0.064769 | 0.074240–0.075218 |
| Record key sort | 2.094850–2.135017 | 0.201892–0.205791 | 1.246099–1.266677 | 0.138958–0.140832 |
| Record comparator sort | 24.724250–24.926625 | 0.185768–0.191432 | 1.182574–1.207588 | 1.878142–1.914167 |
| Growing-copy reduce | 25.775334–25.957542 | 2.381583–2.554950 | 2.749906–2.779865 | 2.594396–2.632521 |
| Node-only mutating reduce | — | 0.006424–0.006899 | — | — |
| Python-only mutating reduce | — | — | — | 0.061989–0.063105 |

### Interpretation

PHP and Python were faster than Scripta on all common workloads in this run.
For 2,048-element list map/filter/reduce, Scripta took about 4.1–4.8 times PHP
and 3.2–4.6 times Python; captured-closure mapping took about 6.7 times PHP
and 5.3 times Python. Node was faster than PHP/Python on most common cases,
while Python's native key sort was the fastest key-sort alternative here.

Within Scripta, key sorting was about 11.8 times faster than comparator sorting
at 2,048 elements. Python's native key sort was about 13.6 times faster than its
comparator adapter. PHP/Node comparator sorting was slightly faster than their
decorated-key adapters at that size on this inexpensive rank lookup. Expensive key callbacks
could produce a different ordering between those alternatives.

Record mapping and comparator sorting show larger Scripta gaps than scalar map,
filter and reduction. Timings alone do not distinguish callback dispatch, record
copying, evaluation, allocation or sorting-algorithm contributions. Small inputs
are especially sensitive to execution-boundary costs and JIT state; use absolute
times alongside ratios. These are workload results, not language rankings.

Growing-copy reduction was costly in all four engines and grew much faster than
input size. Node/Python's additional mutating variants have different accumulator
semantics and should not be treated as paired copying measurements.

Separate empty-process startup medians (ms): Scripta **6.402**, Node **28.855**,
PHP **35.933**, Python **17.956**. These include executable/launcher and CLI
configuration costs, are not API execution times, and are never subtracted.

This run does not show whether adding helpers slowed unrelated code; that needs
matched old/new-revision measurements. Profiling is needed before deciding which
runtime changes would close the observed gaps.

### Evidence and checks

Raw four-runtime evidence is locally retained under
`benchmarks/callbacks/evidence/four-runtimes-2026-11-26/`: shared fixture, all 16
process reports and aggregate `evidence.json`, including all samples and hashes.
Prior Node-only evidence in `local-2026-11-26-final/` is retained separately and
is not mixed into these medians.

New PHP/Python adapters passed syntax checks and explicit LF/CRLF fixtures at
1, 2 and 7 elements. Rust/Node readers already had LF/CRLF validation, and all
four engines passed the full three-size output matrix in this run. Source and
optimized Scripta/Node binary hashes match the measured code; no Rust rebuild
was needed. The existing Rust benchmark and CLI were previously built optimized
and linted; no language implementation or dependencies changed in this follow-up.
Full workspace tests and Clippy were not repeated for the PHP/Python tooling
changes. Other native platforms and release packaging remain unvalidated.

Final evidence audit passed: 6,072 finite positive measured batch samples, 60
startup samples, balanced process order, source/fixture/binary hashes, all
aggregate medians, 19 local documentation paths and diff/whitespace checks.

For the first runtime optimization candidate, see the separate
[owned-argument before/after experiment](owned-arguments.md). Its frozen
baseline/candidate measurements are separate from the cross-language table.
