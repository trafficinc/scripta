# Performance

<!-- BEGIN LANGUAGE BENCHMARKS -->
| Benchmark | Work size | PHP (ms) | Python (ms) | Scripta (ms) |
| --- | --- | ---: | ---: | ---: |
| Numeric loop | 1,000,000 iterations | 49.960 | 112.170 | 32.797 |
| Recursive Fibonacci | fib(28), no memoization | 64.192 | 101.404 | 99.505 |
| List pipeline | 2,000 items × 200 passes | 57.756 | 68.428 | 107.320 |
| String concatenation | 100,000 × 8-byte appends | 828.820 | 44.077 | 24.630 |
| Dictionary operations | 256 keys; 40,960 updates; 128 copy-removals | 38.064 | 47.957 | 42.445 |
| Class dispatch | 200,000 method calls | 42.987 | 79.655 | 95.276 |
| Regex match | 20,000 matches attempted | 37.637 | 67.870 | 16.549 |
| JSON roundtrip | 20,000 encode/decode pairs | 51.915 | 152.568 | 68.964 |
| Function calls | 200,000 calls | 41.377 | 56.232 | 19.499 |
| Startup | Empty script | 35.571 | 34.779 | 5.925 |
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
reuse, guarded list-copy validation, bounded integer self-recursion plans, and
verified ordered-map position hints. General evaluation handles unsupported calls;
the public Value representation and logical resource accounting remain unchanged. See
[runtime ownership and limits](architecture.md#execution-hot-paths-and-limits).

The recursion path retains an accepted increase in execution-only empty-script
overhead. The table above measures whole-process CLI time, including startup.
