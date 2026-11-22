# Performance

<!-- BEGIN LANGUAGE BENCHMARKS -->
| Benchmark | Work size | PHP (ms) | Python (ms) | Scripta (ms) |
| --- | --- | ---: | ---: | ---: |
| Numeric loop | 1,000,000 iterations | 48.920 | 109.811 | 402.862 |
| Recursive Fibonacci | fib(28), no memoization | 62.044 | 101.277 | 435.955 |
| List pipeline | 2,000 items × 200 passes | 55.045 | 67.289 | 168.773 |
| String concatenation | 100,000 × 8-byte appends | 825.968 | 44.064 | 42.086 |
| Dictionary operations | 256 keys; 40,960 updates; 128 copy-removals | 35.898 | 47.739 | 59.385 |
| Class dispatch | 200,000 method calls | 40.994 | 80.030 | 204.872 |
| Regex match | 20,000 matches attempted | 35.156 | 67.620 | 21.877 |
| JSON roundtrip | 20,000 encode/decode pairs | 49.467 | 152.479 | 78.135 |
| Function calls | 200,000 calls | 39.173 | 55.441 | 140.115 |
| Startup | Empty script | 32.977 | 34.526 | 6.011 |
<!-- END LANGUAGE BENCHMARKS -->

The table uses one complete local ARM64 run with 15 samples after 3 warmups per
workload/runtime. Times include process startup; lower is better. See the
[methodology](../benchmarks/comparison/README.md) and
[raw samples, binary identities, compiler, and host details](../benchmarks/comparison/evidence/macos-arm64-20261122-step8.json).
These are workload/API comparisons, not an overall language ranking or native
validation of other platforms.

This build uses the AST evaluator with bounded runtime-local regex caching,
direct method calls, discarded string-buffer reuse, lexical lookup/scope-storage
reuse, and verified ordered-map position hints. The evaluated bytecode prototype
was removed. The public Value representation and logical resource accounting
remain unchanged; see [runtime ownership and limits](architecture.md#execution-hot-paths-and-limits).

In this run, Scripta's regex and startup medians are lower than both comparison
commands. Concatenation is close to Python, and JSON falls between PHP and Python.
Arithmetic loops, recursion, list/dictionary work, and method/function calls remain
slower than both. These whole-process observations include native-library and
startup effects and do not isolate engine throughput.
