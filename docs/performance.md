# Performance

<!-- BEGIN LANGUAGE BENCHMARKS -->
| Benchmark | Work size | PHP (ms) | Python (ms) | Scripta (ms) |
| --- | --- | ---: | ---: | ---: |
| Numeric loop | 1,000,000 iterations | 47.569 | 104.194 | 32.569 |
| Recursive Fibonacci | fib(28), no memoization | 61.755 | 95.954 | 334.161 |
| List pipeline | 2,000 items × 200 passes | 55.154 | 62.464 | 107.650 |
| String concatenation | 100,000 × 8-byte appends | 834.824 | 39.594 | 27.712 |
| Dictionary operations | 256 keys; 40,960 updates; 128 copy-removals | 35.749 | 42.812 | 42.054 |
| Class dispatch | 200,000 method calls | 40.708 | 74.837 | 153.193 |
| Regex match | 20,000 matches attempted | 35.331 | 63.199 | 17.367 |
| JSON roundtrip | 20,000 encode/decode pairs | 49.587 | 148.287 | 69.193 |
| Function calls | 200,000 calls | 39.064 | 50.347 | 19.825 |
| Startup | Empty script | 33.093 | 29.673 | 5.975 |
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
