# Performance

<!-- BEGIN LANGUAGE BENCHMARKS -->
| Benchmark | Work size | PHP (ms) | Python (ms) | Scripta (ms) |
| --- | --- | ---: | ---: | ---: |
| Numeric loop | 1,000,000 iterations | 48.368 | 107.777 | 32.728 |
| Recursive Fibonacci | fib(28), no memoization | 62.532 | 96.984 | 227.049 |
| List pipeline | 2,000 items × 200 passes | 55.559 | 63.211 | 106.891 |
| String concatenation | 100,000 × 8-byte appends | 852.688 | 39.611 | 24.516 |
| Dictionary operations | 256 keys; 40,960 updates; 128 copy-removals | 36.316 | 43.073 | 43.006 |
| Class dispatch | 200,000 method calls | 41.325 | 75.425 | 125.049 |
| Regex match | 20,000 matches attempted | 35.839 | 63.613 | 16.536 |
| JSON roundtrip | 20,000 encode/decode pairs | 50.015 | 149.114 | 69.502 |
| Function calls | 200,000 calls | 40.205 | 51.521 | 19.898 |
| Startup | Empty script | 33.263 | 29.944 | 6.196 |
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
