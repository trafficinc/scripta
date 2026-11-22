# Performance

<!-- BEGIN LANGUAGE BENCHMARKS -->
| Benchmark | Work size | PHP (ms) | Python (ms) | Scripta (ms) |
| --- | --- | ---: | ---: | ---: |
| Numeric loop | 1,000,000 iterations | 47.727 | 106.182 | 53.562 |
| Recursive Fibonacci | fib(28), no memoization | 62.359 | 95.863 | 434.794 |
| List pipeline | 2,000 items × 200 passes | 55.452 | 62.595 | 168.909 |
| String concatenation | 100,000 × 8-byte appends | 840.800 | 39.528 | 42.256 |
| Dictionary operations | 256 keys; 40,960 updates; 128 copy-removals | 36.251 | 43.644 | 59.787 |
| Class dispatch | 200,000 method calls | 41.489 | 75.640 | 205.546 |
| Regex match | 20,000 matches attempted | 35.431 | 62.885 | 21.908 |
| JSON roundtrip | 20,000 encode/decode pairs | 49.616 | 147.913 | 77.456 |
| Function calls | 200,000 calls | 39.402 | 50.387 | 139.420 |
| Startup | Empty script | 33.160 | 29.586 | 6.012 |
<!-- END LANGUAGE BENCHMARKS -->

The table uses one complete local ARM64 run with 15 samples after 3 warmups per
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
