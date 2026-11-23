# Performance

<!-- BEGIN LANGUAGE BENCHMARKS -->
| Benchmark | Work size | PHP (ms) | Python (ms) | Scripta (ms) |
| --- | --- | ---: | ---: | ---: |
| Numeric loop | 1,000,000 iterations | 47.784 | 106.431 | 33.036 |
| Recursive Fibonacci | fib(28), no memoization | 62.700 | 96.626 | 248.257 |
| List pipeline | 2,000 items × 200 passes | 55.746 | 62.558 | 105.070 |
| String concatenation | 100,000 × 8-byte appends | 851.652 | 40.743 | 24.683 |
| Dictionary operations | 256 keys; 40,960 updates; 128 copy-removals | 36.252 | 43.008 | 42.549 |
| Class dispatch | 200,000 method calls | 41.241 | 76.804 | 129.422 |
| Regex match | 20,000 matches attempted | 35.624 | 63.031 | 16.705 |
| JSON roundtrip | 20,000 encode/decode pairs | 49.831 | 149.743 | 69.036 |
| Function calls | 200,000 calls | 39.475 | 51.277 | 19.878 |
| Startup | Empty script | 33.591 | 30.245 | 6.364 |
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
