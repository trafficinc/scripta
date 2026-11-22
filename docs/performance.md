# Performance

<!-- BEGIN LANGUAGE BENCHMARKS -->
| Benchmark | Work size | PHP (ms) | Python (ms) | Scripta (ms) |
| --- | --- | ---: | ---: | ---: |
| Numeric loop | 1,000,000 iterations | 49.283 | 108.201 | 54.501 |
| Recursive Fibonacci | fib(28), no memoization | 66.316 | 100.410 | 443.070 |
| List pipeline | 2,000 items × 200 passes | 60.742 | 67.625 | 173.582 |
| String concatenation | 100,000 × 8-byte appends | 858.404 | 39.978 | 42.309 |
| Dictionary operations | 256 keys; 40,960 updates; 128 copy-removals | 37.472 | 44.118 | 59.715 |
| Class dispatch | 200,000 method calls | 43.696 | 79.444 | 210.665 |
| Regex match | 20,000 matches attempted | 36.297 | 63.912 | 22.094 |
| JSON roundtrip | 20,000 encode/decode pairs | 53.478 | 155.062 | 80.211 |
| Function calls | 200,000 calls | 41.819 | 54.235 | 142.255 |
| Startup | Empty script | 36.206 | 32.057 | 6.984 |
<!-- END LANGUAGE BENCHMARKS -->

The table uses one complete local ARM64 run with 15 samples after 3 warmups per
workload/runtime. Times include process startup; lower is better. See the
[methodology](../benchmarks/comparison/README.md) and
[raw samples, binary identities, compiler, and host details](../benchmarks/comparison/evidence/macos-arm64-20261122-integer-loops.json).
These are workload/API comparisons, not an overall language ranking or native
validation of other platforms.

This build uses the AST evaluator with bounded runtime-local regex caching,
direct method calls, discarded string-buffer reuse, lexical lookup/scope-storage
reuse, and verified ordered-map position hints. The evaluated bytecode prototype
was removed. The public Value representation and logical resource accounting
remain unchanged; see [runtime ownership and limits](architecture.md#execution-hot-paths-and-limits).
