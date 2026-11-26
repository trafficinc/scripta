# Performance

<!-- BEGIN LANGUAGE BENCHMARKS -->
| Benchmark | Work size | PHP (ms) | Python (ms) | Scripta (ms) |
| --- | --- | ---: | ---: | ---: |
| Numeric loop | 1,000,000 iterations | 36.153 | 69.978 | 21.038 |
| Recursive Fibonacci | fib(28), no memoization | 42.970 | 43.367 | 35.090 |
| List pipeline | 2,000 items × 200 passes | 41.673 | 36.650 | 51.480 |
| String concatenation | 100,000 × 8-byte appends | 567.528 | 521.676 | 17.067 |
| Dictionary operations | 256 keys; 40,960 updates; 128 copy-removals | 28.710 | 22.072 | 29.014 |
| Class dispatch | 200,000 method calls | 31.349 | 29.293 | 57.304 |
| Regex match | 20,000 matches attempted | 28.239 | 18.533 | 10.855 |
| JSON roundtrip | 20,000 encode/decode pairs | 39.201 | 61.953 | 52.521 |
| Function calls | 200,000 calls | 30.110 | 28.276 | 15.072 |
| File processing | 20,000 lines; 260,000 bytes | 29.281 | 58.268 | 15.350 |
| CSV → JSON | 10,000 quoted UTF-8 records | 36.130 | 30.833 | 35.744 |
| SQLite transaction | 10,000 inserts + aggregate query (in-memory) | 36.434 | 27.516 | 15.363 |
| Startup | Empty script | 26.989 | 14.323 | 4.496 |
<!-- END LANGUAGE BENCHMARKS -->

The table uses one complete local run with 15 samples after 3 warmups per
workload/runtime. Times include process startup; lower is better. See the
[methodology](../benchmarks/comparison/README.md) for reproduction commands.
Raw samples, binary identities, compiler, and host details are retained locally
in ignored benchmark JSON files.
These are workload/API comparisons, not an overall language ranking or native
validation of other platforms.

The fixed comparison workloads do not include the separate self-recursion,
mutual-recursion, or call-heavy-loop experiments. Fibonacci continues to use its
existing specialized execution path.

This build uses the AST evaluator with bounded runtime-local regex caching,
direct method calls, discarded string-buffer reuse, lexical lookup/scope-storage
reuse, guarded list-copy validation, bounded integer self-recursion plans,
compact instruction plans and guarded heap-backed calls for eligible scalar
functions, adaptive field-read hints,
and verified ordered-map position hints. General evaluation handles unsupported calls;
the public Value representation and logical resource accounting remain unchanged. See
[runtime ownership and limits](architecture.md#execution-hot-paths-and-limits).

Comprehensions reuse bounded uncaptured scope storage and share immutable binding
names. Discarded string appends also reuse their buffer across non-reentrant
native calls whose arguments cannot write script bindings. Ordinary single-field
scalar objects retain their fixed payload size during scalar updates. These
paths preserve copy semantics, callback snapshots, failed writes, and existing
resource checkpoints; other cases retain general evaluation and validation.

The recursion path retains an accepted increase in execution-only empty-script
overhead. The table above measures whole-process CLI time, including startup.

## Callback workloads across runtimes

A separate execution-only comparison on native macOS ARM64 uses optimized
Scripta, Node 22.22.2 with default V8/JIT behavior, PHP 8.4.20 CLI without
OPcache/JIT, and CPython 3.13.18 in isolated mode. Four fresh process rounds
rotate engine order; each measures 11 warmed batches at 128, 2,048 and 8,192
elements. Complete results are checked against a shared reference. Selected
2,048-element medians, in milliseconds:

| Operation | Scripta | Node | PHP | Python |
| --- | ---: | ---: | ---: | ---: |
| List map | 0.240946 | 0.012139 | 0.051679 | 0.076068 |
| Captured-closure map | 0.409225 | 0.012522 | 0.061004 | 0.076723 |
| List filter | 0.342554 | 0.012636 | 0.071711 | 0.074241 |
| List reduce | 0.238423 | 0.009484 | 0.058449 | 0.069393 |
| Record map | 1.241767 | 0.013051 | 0.063999 | 0.074420 |
| Record key sort | 2.103108 | 0.204262 | 1.250070 | 0.139196 |
| Record comparator sort | 24.803083 | 0.186807 | 1.183623 | 1.886208 |
| Growing-copy reduce | 25.846459 | 2.456292 | 2.764135 | 2.597073 |

PHP and Python were faster than Scripta in every common case in this run. For
scalar list map/filter/reduce at this size, Scripta took about 4.1–4.8 times PHP
and 3.2–4.6 times Python. Scripta key sorting was about 11.8 times faster than
its comparator sorting. Python's native key sort was fastest among key-sort
alternatives; PHP and Node use explicit key-decoration adapters.

JavaScript/Python share record references, PHP uses copy-on-write arrays, and
Scripta copies nested collection values and includes execution-boundary and
safety checks. Map transformations also use explicit adapters with different
representations. Timings do not isolate the cost of any one implementation choice.
Growing-collection reduction is expensive in all copying variants; additional
Node/Python mutating baselines have different semantics.

Separate empty-process startup medians were Scripta 6.402 ms, Node 28.855 ms,
PHP 35.933 ms and Python 17.956 ms. Startup is never subtracted from execution.
See the [complete results and methodology](../benchmarks/callbacks/README.md)
for all sizes, repeated-process ranges, runtime settings, reproduction and local
evidence. This run does not establish old/new-revision regressions or validate
other native platforms.

The cross-runtime table above records the pre-optimization reference. The
[owned-argument experiment](../benchmarks/callbacks/owned-arguments.md) measures
the first optimization candidate against a preserved build of that reference.
Candidate acceptance is separate from these measurements.

The [callback-setup experiment](../benchmarks/callbacks/callback-setup.md) measures
the next candidate incrementally against accepted step 1. Scope/value quotas and
unsupported callback forms keep their existing execution path.

The [argument-validation experiment](../benchmarks/callbacks/argument-validation.md)
measures the next candidate against accepted step 2. It reuses structural checks
only for small immutable borrowed data during repeated comparisons; mutable
objects, callable descendants and byte-budgeted invocations retain full checks.
