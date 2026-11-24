# Performance

<!-- BEGIN LANGUAGE BENCHMARKS -->
| Benchmark | Work size | PHP (ms) | Python (ms) | Scripta (ms) |
| --- | --- | ---: | ---: | ---: |
| Numeric loop | 1,000,000 iterations | 48.056 | 103.874 | 32.632 |
| Recursive Fibonacci | fib(28), no memoization | 61.669 | 95.158 | 57.830 |
| List pipeline | 2,000 items × 200 passes | 55.355 | 62.521 | 106.354 |
| String concatenation | 100,000 × 8-byte appends | 825.300 | 38.773 | 24.599 |
| Dictionary operations | 256 keys; 40,960 updates; 128 copy-removals | 35.922 | 42.579 | 42.463 |
| Class dispatch | 200,000 method calls | 40.889 | 74.647 | 94.170 |
| Regex match | 20,000 matches attempted | 35.513 | 63.060 | 16.543 |
| JSON roundtrip | 20,000 encode/decode pairs | 49.388 | 147.250 | 69.457 |
| Function calls | 200,000 calls | 39.022 | 50.522 | 19.525 |
| Startup | Empty script | 33.047 | 30.136 | 6.231 |
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
