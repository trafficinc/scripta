# Performance and safety review — Phase 13

## Measurement environment and method

Measured on 2026-11-12: macOS-27.0.1-arm64-arm-64bit, 8 logical CPUs,
Rust 1.99.1, default Cargo release profile (no custom LTO/strip settings),
Python 3.9.6, and GNU bash, version 3.2.57(1)-release (arm64-apple-darwin26).
Only macOS arm64 measurements are reported. Linux/Windows have CI compatibility
coverage; this report contains no Raspberry Pi, VPS, Linux, or Windows estimates.

The runner takes two warmups followed by 11 measured launches for each fixture/
language, validates every output checksum, and records every observation plus
minimum/maximum/median wall time. The final run measures the pre-Phase-13 reference
binary and the optimized binary in the same invocation. This is warm-cache CLI
latency including process creation, file loading, parsing, runtime setup, execution,
and cleanup, not cold-boot latency or an isolated CPU benchmark. Fixtures are small
and deliberately repeatable; these numbers do not characterize every program.

Peak RSS comes from `wait4`/`ru_maxrss` for each child, using bytes on macOS and KiB
on Linux, converted to MiB. Tables show the largest observation among the 11 runs.
This excludes the Python driver and is not the sum of concurrent descendants'
resident memory. On platforms without `wait4`, RSS is explicitly unavailable.
No memory metric is inferred from wall time or cumulative child high-water marks.

## CLI execution measurements

All values below are observed median milliseconds, including startup. “Before” is
the preserved release binary from the Phase 12 working tree, identified by hash
in the raw data. No benchmark source contains optional Phase 13 limit settings.

| Fixture | Scripta before | Scripta after | Bash | Python |
| --- | ---: | ---: | ---: | ---: |
| Empty script startup | 5.605 | 6.372 | 2.911 | 18.097 |
| 20,000 variable accumulations | 13.791 | 13.791 | 51.942 | 19.741 |
| 10,000 function calls | 12.496 | 12.786 | 50.437 | 19.682 |
| 150 × 150 nested loops | 14.108 | 14.690 | 56.417 | 20.150 |
| 12,000 string appends | 63.963 | 10.566 | 52.587 | 20.006 |
| 12,000 list reads (512 elements) | 31.751 | 12.443 | 44.513 | 20.572 |
| 128 inserts + 8,000 map reads | 31.963 | 12.037 | N/A | 19.608 |
| Eight reads of a 1 MiB ASCII file | 7.832 | 7.588 | 123.888 | 21.148 |
| 20 direct child executions | 306.646 | 158.630 | 37.521 | 63.503 |
| 49 retained copies of 4,096 integers | 60.539 | 10.235 | N/A | 79.664 |

| Fixture | Before peak MiB | After peak MiB | Bash peak MiB | Python peak MiB |
| --- | ---: | ---: | ---: | ---: |
| startup | 6.547 | 6.641 | 2.016 | 7.688 |
| variables | 6.656 | 6.734 | 2.141 | 7.656 |
| calls | 6.703 | 6.766 | 2.188 | 7.672 |
| loops | 6.672 | 6.734 | 2.141 | 7.672 |
| strings | 8.141 | 6.984 | 3.547 | 7.875 |
| lists | 6.797 | 6.859 | 2.234 | 8.375 |
| maps | 7.016 | 6.812 | N/A | 7.688 |
| files | 8.750 | 8.812 | 12.625 | 9.703 |
| processes | 7.406 | 7.453 | 2.094 | 9.375 |
| memory | 31.656 | 19.625 | N/A | 11.047 |

Bash 3.2 has no associative or nested arrays, so map and retained-nested-copy
comparisons are unavailable rather than approximated. Bash's function fixture
uses a global result to avoid introducing a subshell per call. Its file fixture
uses `cat` in command substitution; Scripta and Python use their file APIs. All
file data is ASCII with no trailing newline. The memory fixture deliberately
copies values in Python with `deepcopy`, since ordinary Python assignment shares
references while Scripta assignment has value semantics. The process fixture
executes the discovered `true` executable directly; Scripta additionally pays for
its process-group/job cleanup, bounded capture files, and monitor. These language
idioms have equivalent checked outputs, not identical implementation costs.

The final paired retained-copy fixture observed 31.656 MiB before and
19.625 MiB after. The initial baseline observed 34.438 MiB, and exploratory
runs also produced lower baseline peaks. RSS is sensitive to allocator/OS state;
these observations do not promise a fixed reduction or a process memory ceiling.
The consistent gains are execution time for concatenation, indexed collections,
and process waits. Ordinary scalar/call/loop timings remain close, with some
small regressions visible in the table; no universal speedup is claimed.

## Release binary size

| Artifact | Bytes | MiB |
| --- | ---: | ---: |
| Before | 3,923,504 | 3.742 |
| After | 3,985,072 | 3.800 |

This is the actual macOS executable on disk, not compressed download size. The
small increase includes limits, redaction, validation, and wait notifications.
Binary SHA-256 values are recorded in the JSON results.

## Isolated phases and representation experiment

The Rust harness parses each fixture once for the execution measurement. Runtime
construction and standard-library registration happen outside that timer; function
declaration registration and execution remain inside it. Lex/parse rows each use
1,000 observations; execution rows use nine. The table reports median milliseconds.
Very small startup phase results approach the timer's resolution.

| Fixture | Lex after | Parse after | Execute before | Execute after |
| --- | ---: | ---: | ---: | ---: |
| startup | 0.000167 | 0.000042 | 0.000166 | 0.000084 |
| variables | 0.002917 | 0.001542 | 7.488875 | 7.156625 |
| calls | 0.003750 | 0.001750 | 5.790417 | 5.732209 |
| loops | 0.003667 | 0.001833 | 7.630541 | 7.619208 |
| strings | 0.002875 | 0.001375 | 60.388166 | 3.974875 |
| lists | 0.009917 | 0.001792 | 25.479542 | 5.841375 |
| maps | 0.005375 | 0.002625 | 25.553375 | 5.487917 |

A separate one-batch model measures 10,000 snapshots of a 4,096-element
`Vec<Value>` (total milliseconds, not runtime medians):

| Model | Observed ms |
| --- | ---: |
| owned_clone | 124.157125 |
| rc_clone | 0.040917 |
| cow_write | 124.264666 |

Reference counting substantially reduces read-only snapshot cost. `Rc::make_mut`
when every snapshot is written must copy, and costs approximately as much as the
owned-clone model. Copy-on-write could help alias-heavy workloads and large
collection arguments such as repeated `len($list)`, but changing the public Value
representation would affect every native module and host consumer. The measured
hot paths can be improved without that change, so owned values remain. Map lookup
is still linear and equality is still order-insensitive with quadratic matching;
no VM, JIT, hash-map representation migration, or new collector was introduced.

## Implemented targeted changes

- Variable-root index traversal borrows collections and clones only the selected
  value. Indices with calls, assignment, or native property effects retain the
  original snapshot evaluation path; error/evaluation order is tested.
- Indexed variable writes mutate the owned root directly, preserving independent
  copies held by other variables. Explicit byte budgets use a transactional
  candidate so a rejected update does not partially modify the original value.
- Discarded assignment statements move their result instead of cloning it.
  Pure-RHS string `.=` statements reuse the existing buffer; effectful RHS and
  assignments whose result is used retain the general path.
- Map validation uses hashed strictly typed keys instead of repeated linear
  duplicate searches. Value validation is iterative and depth-bounded, with a
  fast scalar path.
- Process waiters receive a condition-variable notification on completion instead
  of imposing a second polling-delay floor. The monitor retains its 5 ms poll and
  timeout/cancellation/output checks; a timed wait keeps cancellation observable.

## Safety audit and execution limits

| Requirement | Verification or change |
| --- | --- |
| No unsafe workspace Rust | Every package inherits workspace `unsafe_code = "forbid"`, including tests/benches. No unsafe source was introduced. Third-party platform/crypto dependencies may internally use unsafe Rust/FFI; this is not a claim about their internals. |
| No unnecessary execution AST clones | Function calls borrow their stored declaration. A declaration is cloned when registered because its callable must survive the caller dropping Program or an included AST; that ownership clone remains necessary under the public borrowed-Program API. |
| Bounded recursion | Parser nesting 64, expression nodes 256, calls/includes at most 64, runtime value nesting at most 128. Script-created collections are validated before growing beyond this limit. Trusted Rust hosts must not construct pathologically deep ASTs/values outside parser/runtime contracts. |
| Checked arithmetic | Integer arithmetic/negation/division/remainder use checked operations, including MIN / -1. Finite float results are required. Size accounting and string reservations check overflow/allocation errors. |
| Bounded subprocess capture | Default 8 MiB combined stdout/stderr, host ceiling, 60 s default timeout, direct argv, owned children, process-group/job termination and reaping. Temporary disk use may overshoot between monitor polls; this is not an OS disk quota. |
| Bounded HTTP responses | Default 8 MiB, enforced for known-length and chunked bodies, host ceiling, 30 s timeout, verified TLS, five redirects. Existing fixture tests cover errors and boundaries. |
| Malformed source | Parser depth/complexity limits, 512-case Unicode/token-noise properties, explicit 10,000-level malformed inputs, formatter token checks; no observed panics in these tests. Allocator/OS exhaustion is not a recoverable-language guarantee. |
| No implicit shell through Process::run | Literal argv tests include shell metacharacters. Shell execution is separately named and permission-gated; Windows batch files are rejected by direct run/spawn. |
| Diagnostic credentials | Source excerpts mask all string/comment contents while preserving scalar columns. Known sensitive environment values and host-supplied redacted values are removed from error text/path rendering. Credential-shaped assignments, Bearer values, PEM headers, and URL userinfo are redacted. Debug errors do not dump source text or the redaction list. |
| Strict values | Existing tests cover mixed int/float rejection, bool-only conditions, explicit conversions, and snapshot semantics. These semantics are retained. |
| Configurable budgets | Instructions, per-value logical bytes, live lexical scopes, source bytes, call depth, subprocess/HTTP bounds, permission and filesystem policies. |
| Cleanup after failure | Tests cover unreferenced native-resource drops on exception, scope/stack reuse, failed process setup, timeout/cancellation, output limits, handle drop, descendant cleanup, and CLI unwind. CLI runtimes drop on every result. |

### Budget configuration

```sh
SCR_MAX_STEPS=1000000 SCR_MAX_VALUE_BYTES=8388608 SCR_MAX_SCOPES=1024 \
SCR_MAX_SOURCE_BYTES=1048576 scr run automation.scr
```

`RuntimeOptions` exposes the same `instruction_limit`, `max_value_bytes`,
`max_scopes`, and `max_source_bytes`, plus `recursion_limit`. Source defaults to
8 MiB per entry/include; the other three new CLI budgets are opt-in to preserve
ordinary script behavior. Invalid budget environment values exit 2 without
printing their contents; exceeded budgets exit 1. Value depth is a hard 128.
Instructions count statement/loop checkpoints, not CPU instructions or blocking
wall time. Includes share the existing execution budget rather than resetting it.

The byte limit counts each Value slot, string UTF-8 bytes, map-key slots/strings,
and nested values. It is a **logical per-value budget**, not an aggregate process
heap/RSS limit: allocator capacity, temporary candidates, AST/source copies,
opaque native-object internals, and other values can use additional memory.
Configured File::read checks its growing buffer before appending beyond the value
budget; File::copy already streams. Without a configured byte limit, File::read
retains its existing whole-file behavior. OS/container memory limits are still
needed when enforcing a hard process memory ceiling.

Captured lexical environments remain in the runtime arena until runtime drop;
unreachable closures are not garbage-collected during a long-lived runtime.
The new live-scope ceiling bounds that retention when configured. Resources still
referenced by globals/captured environments intentionally remain owned until those
references/runtime are dropped; an exception does not erase globals. Uncaptured
transient scopes unwind immediately. No daemon or new GC architecture is added.

Diagnostic redaction is conservative and can hide benign credential-shaped text.
It cannot identify every arbitrary unmarked secret embedded in a user-thrown
message; callers should avoid throwing credentials and hosts can configure
`ContextOptions.redacted_values`. Script catch values retain their original value;
redaction affects reporting, not control flow or string operations. Raw host access
to SourceFile.text is trusted and intentionally still available.

## Reproduction and evidence

See [benchmark instructions](../benchmarks/README.md). Raw evidence includes
[initial baseline](../benchmarks/results/baseline.json),
[final paired results](../benchmarks/results/optimized.json),
[baseline phases](../benchmarks/results/baseline-phases.csv), and
[optimized phases/sharing](../benchmarks/results/optimized-phases.csv).
The JSON includes individual observations, executable identities and fixture hashes.
Preserved local benchmark executables are ignored by Git; no binary is required
for rerunning the optimized fixtures from source.
