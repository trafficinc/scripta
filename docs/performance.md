# Performance

<!-- BEGIN LANGUAGE BENCHMARKS -->
| Benchmark | Work size | PHP (ms) | Python (ms) | Scripta (ms) |
| --- | --- | ---: | ---: | ---: |
| Numeric loop | 1,000,000 iterations | 48.661 | 109.101 | 33.240 |
| Recursive Fibonacci | fib(28), no memoization | 61.994 | 95.967 | 96.306 |
| List pipeline | 2,000 items × 200 passes | 55.463 | 62.648 | 106.623 |
| String concatenation | 100,000 × 8-byte appends | 832.583 | 39.381 | 24.968 |
| Dictionary operations | 256 keys; 40,960 updates; 128 copy-removals | 35.675 | 42.394 | 42.463 |
| Class dispatch | 200,000 method calls | 42.240 | 77.746 | 96.707 |
| Regex match | 20,000 matches attempted | 35.308 | 62.963 | 16.644 |
| JSON roundtrip | 20,000 encode/decode pairs | 50.545 | 151.604 | 70.687 |
| Function calls | 200,000 calls | 39.305 | 50.486 | 19.574 |
| Startup | Empty script | 32.902 | 29.330 | 6.047 |
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
reuse, guarded list-copy validation, bounded integer self-recursion plans,
compact instruction plans for eligible scalar functions, adaptive field-read hints,
and verified ordered-map position hints. General evaluation handles unsupported calls;
the public Value representation and logical resource accounting remain unchanged. See
[runtime ownership and limits](architecture.md#execution-hot-paths-and-limits).

The recursion path retains an accepted increase in execution-only empty-script
overhead. The table above measures whole-process CLI time, including startup.
