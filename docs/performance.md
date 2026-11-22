# Performance

<!-- BEGIN LANGUAGE BENCHMARKS -->
| Benchmark | Work size | PHP (ms) | Python (ms) | Scripta (ms) |
| --- | --- | ---: | ---: | ---: |
| Numeric loop | 1,000,000 iterations | 51.006 | 109.221 | 54.440 |
| Recursive Fibonacci | fib(28), no memoization | 65.757 | 99.316 | 344.360 |
| List pipeline | 2,000 items × 200 passes | 55.977 | 62.975 | 109.949 |
| String concatenation | 100,000 × 8-byte appends | 859.756 | 40.014 | 27.761 |
| Dictionary operations | 256 keys; 40,960 updates; 128 copy-removals | 37.548 | 45.017 | 43.011 |
| Class dispatch | 200,000 method calls | 43.679 | 78.227 | 155.270 |
| Regex match | 20,000 matches attempted | 37.889 | 68.663 | 18.231 |
| JSON roundtrip | 20,000 encode/decode pairs | 51.010 | 152.103 | 69.994 |
| Function calls | 200,000 calls | 39.675 | 50.918 | 93.590 |
| Startup | Empty script | 33.548 | 29.543 | 6.171 |
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
