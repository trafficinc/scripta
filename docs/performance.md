# Performance

<!-- BEGIN LANGUAGE BENCHMARKS -->
| Benchmark | Work size | PHP (ms) | Python (ms) | Scripta (ms) |
| --- | --- | ---: | ---: | ---: |
| Numeric loop | 1,000,000 iterations | 47.542 | 105.495 | 32.521 |
| Recursive Fibonacci | fib(28), no memoization | 61.828 | 95.810 | 333.181 |
| List pipeline | 2,000 items × 200 passes | 55.331 | 62.655 | 107.580 |
| String concatenation | 100,000 × 8-byte appends | 836.302 | 39.395 | 27.407 |
| Dictionary operations | 256 keys; 40,960 updates; 128 copy-removals | 35.795 | 42.669 | 41.335 |
| Class dispatch | 200,000 method calls | 40.763 | 75.312 | 152.061 |
| Regex match | 20,000 matches attempted | 35.003 | 62.777 | 17.132 |
| JSON roundtrip | 20,000 encode/decode pairs | 49.194 | 147.837 | 68.772 |
| Function calls | 200,000 calls | 38.806 | 49.783 | 25.582 |
| Startup | Empty script | 32.888 | 29.877 | 6.119 |
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
