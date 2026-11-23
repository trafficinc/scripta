# Performance

<!-- BEGIN LANGUAGE BENCHMARKS -->
| Benchmark | Work size | PHP (ms) | Python (ms) | Scripta (ms) |
| --- | --- | ---: | ---: | ---: |
| Numeric loop | 1,000,000 iterations | 48.046 | 107.270 | 33.079 |
| Recursive Fibonacci | fib(28), no memoization | 62.196 | 97.002 | 341.342 |
| List pipeline | 2,000 items × 200 passes | 55.263 | 63.182 | 109.627 |
| String concatenation | 100,000 × 8-byte appends | 834.778 | 39.181 | 27.474 |
| Dictionary operations | 256 keys; 40,960 updates; 128 copy-removals | 35.868 | 42.941 | 42.574 |
| Class dispatch | 200,000 method calls | 40.778 | 74.766 | 155.827 |
| Regex match | 20,000 matches attempted | 34.982 | 62.473 | 16.871 |
| JSON roundtrip | 20,000 encode/decode pairs | 49.300 | 148.186 | 69.286 |
| Function calls | 200,000 calls | 39.067 | 50.522 | 19.666 |
| Startup | Empty script | 34.468 | 31.383 | 6.521 |
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
