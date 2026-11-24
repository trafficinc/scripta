# Performance

<!-- BEGIN LANGUAGE BENCHMARKS -->
| Benchmark | Work size | PHP (ms) | Python (ms) | Scripta (ms) |
| --- | --- | ---: | ---: | ---: |
| Numeric loop | 1,000,000 iterations | 49.094 | 107.718 | 32.491 |
| Recursive Fibonacci | fib(28), no memoization | 61.717 | 95.242 | 96.133 |
| List pipeline | 2,000 items × 200 passes | 56.144 | 63.306 | 107.050 |
| String concatenation | 100,000 × 8-byte appends | 837.630 | 39.103 | 24.563 |
| Dictionary operations | 256 keys; 40,960 updates; 128 copy-removals | 36.075 | 42.633 | 42.236 |
| Class dispatch | 200,000 method calls | 42.069 | 76.949 | 95.334 |
| Regex match | 20,000 matches attempted | 36.034 | 65.336 | 16.892 |
| JSON roundtrip | 20,000 encode/decode pairs | 51.582 | 152.852 | 70.414 |
| Function calls | 200,000 calls | 41.912 | 54.448 | 20.423 |
| Startup | Empty script | 35.875 | 32.887 | 7.075 |
<!-- END LANGUAGE BENCHMARKS -->

The table uses one complete local run with 15 samples after 3 warmups per
workload/runtime. Times include process startup; lower is better. See the
[methodology](../benchmarks/comparison/README.md) for reproduction commands.
Raw samples, binary identities, compiler, and host details are retained locally
in ignored benchmark JSON files.
These are workload/API comparisons, not an overall language ranking or native
validation of other platforms.

The fixed comparison workloads do not include the separate self-recursion,
mutual-recursion, or call-heavy-loop experiments. Fibonacci continues to use its
existing specialized execution path.

This build uses the AST evaluator with bounded runtime-local regex caching,
direct method calls, discarded string-buffer reuse, lexical lookup/scope-storage
reuse, guarded list-copy validation, bounded integer self-recursion plans,
compact instruction plans and guarded heap-backed calls for eligible scalar
functions, adaptive field-read hints,
and verified ordered-map position hints. General evaluation handles unsupported calls;
the public Value representation and logical resource accounting remain unchanged. See
[runtime ownership and limits](architecture.md#execution-hot-paths-and-limits).

The recursion path retains an accepted increase in execution-only empty-script
overhead. The table above measures whole-process CLI time, including startup.
