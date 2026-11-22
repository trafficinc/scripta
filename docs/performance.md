# Performance

<!-- BEGIN LANGUAGE BENCHMARKS -->
| Benchmark | Work size | PHP (ms) | Python (ms) | Scripta (ms) |
| --- | --- | ---: | ---: | ---: |
| Numeric loop | 1,000,000 iterations | 47.691 | 104.311 | 53.151 |
| Recursive Fibonacci | fib(28), no memoization | 62.280 | 96.550 | 358.164 |
| List pipeline | 2,000 items × 200 passes | 55.291 | 62.507 | 109.226 |
| String concatenation | 100,000 × 8-byte appends | 830.993 | 39.350 | 27.778 |
| Dictionary operations | 256 keys; 40,960 updates; 128 copy-removals | 36.176 | 42.884 | 42.537 |
| Class dispatch | 200,000 method calls | 41.049 | 75.207 | 160.744 |
| Regex match | 20,000 matches attempted | 35.493 | 62.775 | 17.685 |
| JSON roundtrip | 20,000 encode/decode pairs | 49.380 | 147.610 | 69.605 |
| Function calls | 200,000 calls | 39.319 | 50.478 | 98.728 |
| Startup | Empty script | 33.211 | 29.580 | 6.051 |
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
