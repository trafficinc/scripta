# Performance

<!-- BEGIN LANGUAGE BENCHMARKS -->
| Benchmark | Work size | PHP (ms) | Python (ms) | Scripta (ms) |
| --- | --- | ---: | ---: | ---: |
| Numeric loop | 1,000,000 iterations | 36.353 | 71.409 | 22.276 |
| Recursive Fibonacci | fib(28), no memoization | 43.167 | 44.519 | 35.162 |
| List pipeline | 2,000 items × 200 passes | 41.297 | 37.431 | 64.800 |
| String concatenation | 100,000 × 8-byte appends | 566.285 | 528.464 | 17.349 |
| Dictionary operations | 256 keys; 40,960 updates; 128 copy-removals | 29.001 | 22.231 | 29.571 |
| Class dispatch | 200,000 method calls | 31.574 | 29.733 | 70.042 |
| Regex match | 20,000 matches attempted | 27.928 | 18.713 | 11.328 |
| JSON roundtrip | 20,000 encode/decode pairs | 39.201 | 62.945 | 53.096 |
| Function calls | 200,000 calls | 30.345 | 28.135 | 15.245 |
| File processing | 20,000 lines; 260,000 bytes | 29.473 | 58.637 | 15.454 |
| CSV → JSON | 10,000 quoted UTF-8 records | 36.054 | 31.213 | 35.673 |
| SQLite transaction | 10,000 inserts + aggregate query (in-memory) | 35.588 | 27.317 | 15.243 |
| Startup | Empty script | 26.832 | 14.448 | 4.413 |
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
