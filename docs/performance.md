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
