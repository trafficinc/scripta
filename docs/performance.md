# Performance

<!-- BEGIN LANGUAGE BENCHMARKS -->
| Benchmark | Work size | PHP (ms) | Python (ms) | Scripta (ms) |
| --- | --- | ---: | ---: | ---: |
| Numeric loop | 1,000,000 iterations | 49.172 | 107.856 | 54.017 |
| Recursive Fibonacci | fib(28), no memoization | 63.967 | 98.316 | 346.728 |
| List pipeline | 2,000 items × 200 passes | 56.048 | 63.329 | 110.847 |
| String concatenation | 100,000 × 8-byte appends | 848.982 | 39.825 | 27.538 |
| Dictionary operations | 256 keys; 40,960 updates; 128 copy-removals | 35.966 | 42.848 | 42.369 |
| Class dispatch | 200,000 method calls | 40.867 | 75.767 | 162.039 |
| Regex match | 20,000 matches attempted | 37.222 | 66.509 | 17.753 |
| JSON roundtrip | 20,000 encode/decode pairs | 52.165 | 153.933 | 71.449 |
| Function calls | 200,000 calls | 39.755 | 51.375 | 95.052 |
| Startup | Empty script | 33.570 | 30.597 | 6.292 |
<!-- END LANGUAGE BENCHMARKS -->

The table uses one complete local run with 15 samples after 3 warmups per
workload/runtime. Times include process startup; lower is better. See the
[methodology](../benchmarks/comparison/README.md) for reproduction commands.
Raw samples, binary identities, compiler, and host details are retained locally
in ignored benchmark JSON files.
These are workload/API comparisons, not an overall language ranking or native
validation of other platforms.

This build uses the AST evaluator with bounded runtime-local regex caching,
direct method calls, discarded string-buffer reuse, lexical lookup/scope-storage
reuse, and verified ordered-map position hints. The evaluated bytecode prototype
was removed. The public Value representation and logical resource accounting
remain unchanged; see [runtime ownership and limits](architecture.md#execution-hot-paths-and-limits).
