# Performance

<!-- BEGIN LANGUAGE BENCHMARKS -->
| Benchmark | Work size | PHP (ms) | Python (ms) | Scripta (ms) |
| --- | --- | ---: | ---: | ---: |
| Numeric loop | 1,000,000 iterations | 47.492 | 103.973 | 32.708 |
| Recursive Fibonacci | fib(28), no memoization | 62.104 | 95.468 | 224.603 |
| List pipeline | 2,000 items × 200 passes | 54.965 | 62.231 | 105.148 |
| String concatenation | 100,000 × 8-byte appends | 820.818 | 39.366 | 24.208 |
| Dictionary operations | 256 keys; 40,960 updates; 128 copy-removals | 35.908 | 42.284 | 41.826 |
| Class dispatch | 200,000 method calls | 40.551 | 74.802 | 93.820 |
| Regex match | 20,000 matches attempted | 35.044 | 62.858 | 15.934 |
| JSON roundtrip | 20,000 encode/decode pairs | 49.421 | 147.528 | 68.827 |
| Function calls | 200,000 calls | 38.865 | 50.264 | 18.989 |
| Startup | Empty script | 32.938 | 29.479 | 5.828 |
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
reuse, guarded list-copy validation, and verified ordered-map position hints.
The evaluated bytecode prototype was removed. The public Value representation
and logical resource accounting remain unchanged; see
[runtime ownership and limits](architecture.md#execution-hot-paths-and-limits).
