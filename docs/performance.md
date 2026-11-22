# Performance

<!-- BEGIN LANGUAGE BENCHMARKS -->
| Benchmark | Work size | PHP (ms) | Python (ms) | Scripta (ms) |
| --- | --- | ---: | ---: | ---: |
| Numeric loop | 1,000,000 iterations | 47.777 | 105.235 | 32.919 |
| Recursive Fibonacci | fib(28), no memoization | 62.013 | 95.803 | 330.620 |
| List pipeline | 2,000 items × 200 passes | 54.875 | 61.971 | 107.725 |
| String concatenation | 100,000 × 8-byte appends | 830.190 | 39.197 | 27.424 |
| Dictionary operations | 256 keys; 40,960 updates; 128 copy-removals | 35.905 | 42.512 | 41.549 |
| Class dispatch | 200,000 method calls | 40.828 | 75.122 | 152.284 |
| Regex match | 20,000 matches attempted | 35.262 | 62.658 | 17.222 |
| JSON roundtrip | 20,000 encode/decode pairs | 49.622 | 147.502 | 68.823 |
| Function calls | 200,000 calls | 38.956 | 50.278 | 93.428 |
| Startup | Empty script | 32.979 | 29.492 | 5.893 |
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
