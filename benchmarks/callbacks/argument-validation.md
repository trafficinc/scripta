# Argument validation experiment: step 3

Status: implemented and measured; acceptance pending. Step 2 is accepted.
This experiment is incremental against accepted step 2. Broader execution plans
and copy-on-write collections remain separate future steps.

## Change and proof boundary

Reentrant natives can retain an InvocationArgument borrowing an input Value and
invoke a callback through InvocationContext::invoke_arguments. The constructor
does no validation. Each invocation still clones argument snapshots before the
ordinary checkpoint and callable check. The first actual use validates every
argument in its original order, before any callback code runs.

Only successful checks can be reused, only without a configured byte quota, and
only for data trees containing at most 64 Value nodes. Allowed leaves are null,
bool, int, finite float, owned strings and immutable bytes. Lists/maps may contain
those data values. Every object or callable descendant forces fresh validation;
larger trees also remain uncached. Classification is bounded to 64 enum visits
and never calls native hooks. The allowed Value variants are exhaustively matched
so a future mutable variant cannot silently qualify.

Rust's immutable borrow fixes the underlying owned collection data for the
holder's lifetime. Callback writes affect cloned arguments, and shared objects
cannot enter the reusable case. No global pointer cache, public trusted bit,
callback-result memo, skipped annotation, new execution body form or COW is used.
Changing to a byte-budgeted context forces full validation even after a previous
unbudgeted success. Owner/arity/signature, call/scope/instruction limits, typed
throws, exits, stack/source restoration and output checks retain their paths.

List::sortWith opts in when byte quotas are unset and the first input is a list
or map. Holders are prepared during the existing input preflight, after each
visited item's checks; its checkpoint/prefix-budget order is unchanged. Metadata
uses fallible geometric growth, is bounded by visited entries, and allocation
failure drops it and falls back to ordinary invocation. Holders are dropped
before output measurement/copying. Key sorting, scalar-first inputs, byte-budgeted
sorting and the other collection helpers keep their ordinary path. Mapping visits
each snapshot once and would gain no reuse from these holders.

Temporary holder capacity can exceed its current length through Vec growth;
logical value limits remain distinct from a process heap cap. The sorting cache
is not allocated with a configured byte quota. Shared/mutable native payloads are
remeasured as before, including original hook order and alias counts.

## Reproduce

Preserve the accepted step-2 benchmark engine and default-feature optimized CLI
before rebuilding. Build the candidate using the pinned native compiler/profile
and the commands in the [callback comparison](README.md). Use a fresh output path:

```sh
python3 benchmarks/callbacks/owned_arguments.py --baseline BASELINE_ENGINE --candidate CANDIDATE_ENGINE --baseline-cli BASELINE_CLI --candidate-cli CANDIDATE_CLI --source crates/scr-runtime/src/lib.rs --source crates/scr-runtime/src/value.rs --source crates/scr-runtime/src/invocation/argument_tests.rs --source crates/scr-stdlib/src/collections/callbacks/tests.rs --output benchmarks/callbacks/evidence/argument-validation-local
```

Four fresh process pairs alternate old/new order. Identical fixtures/callbacks
cover 128/2048/8192 entries, at least 100 ms warmup per case and 11 calibrated
batches (target 10 ms, maximum 10000 operations). Full final results are checked
outside every batch. Parsing, registration, callback definitions, fixture creation
and verification are outside timing; execution, validation, clones, callbacks,
assignments and normal cleanup are inside. Tables summarize four process medians;
all raw samples, batch counts, ranges, hashes, compiler/host and the working-tree
patch are retained. Build/test work finishes before measurement. No timing
thresholds, forced GC or heap-profiler measurements are used.

Separate CLI controls use 3 warmups and 15 alternating measured samples for an
empty script and the existing numeric_loop, function_calls and list_pipeline
scripts. These include process launch, parsing and setup; startup is not subtracted
or mixed with execution-only callback timings. Complete outputs are checked.

## Results

Incremental baseline: accepted step 2 at revision
46977e19d251a2827088a4ba099c6ee97f8305ef. Baseline binaries match that step's
final evidence. Fresh pre-edit source hashes match HEAD. Final candidate binaries
were rebuilt after validation and frozen separately from the preliminary trial.
The final revision narrows the trial's input-preparation path so key sorts,
scalar-first sorts and byte-budgeted sorts keep their original preflight.

All full-output checks passed. Median milliseconds below summarize four process
medians. Positive saved time means less elapsed time; negative means slower.

| Elements | Operation | Accepted step 2 ms | Step 3 candidate ms | Time saved |
| ---: | --- | ---: | ---: | ---: |
| 128 | list_map | 0.037945 | 0.038111 | -0.43% |
| 128 | list_map_capture | 0.050107 | 0.050210 | -0.20% |
| 128 | list_filter | 0.046003 | 0.046088 | -0.19% |
| 128 | list_reduce | 0.037048 | 0.037453 | -1.09% |
| 128 | map_map | 0.055158 | 0.055401 | -0.44% |
| 128 | map_filter | 0.056218 | 0.056967 | -1.33% |
| 128 | map_reduce | 0.046639 | 0.047213 | -1.23% |
| 128 | record_map | 0.100006 | 0.100071 | -0.06% |
| 128 | record_sort_key | 0.152357 | 0.153194 | -0.55% |
| 128 | record_sort_compare | 0.755298 | 0.589714 | +21.92% |
| 128 | reduce_growing_copy | 0.144326 | 0.144559 | -0.16% |
| 2048 | list_map | 0.217329 | 0.219360 | -0.93% |
| 2048 | list_map_capture | 0.408285 | 0.408951 | -0.16% |
| 2048 | list_filter | 0.339710 | 0.345130 | -1.60% |
| 2048 | list_reduce | 0.206815 | 0.212846 | -2.92% |
| 2048 | map_map | 0.483514 | 0.488135 | -0.96% |
| 2048 | map_filter | 0.500801 | 0.510089 | -1.85% |
| 2048 | map_reduce | 0.360021 | 0.365509 | -1.52% |
| 2048 | record_map | 1.239688 | 1.245581 | -0.48% |
| 2048 | record_sort_key | 2.087613 | 2.089850 | -0.11% |
| 2048 | record_sort_compare | 20.007312 | 15.206167 | +24.00% |
| 2048 | reduce_growing_copy | 18.294583 | 18.517625 | -1.22% |
| 8192 | list_map | 0.782178 | 0.799864 | -2.26% |
| 8192 | list_map_capture | 1.545083 | 1.558818 | -0.89% |
| 8192 | list_filter | 1.273380 | 1.292124 | -1.47% |
| 8192 | list_reduce | 0.747127 | 0.776049 | -3.87% |
| 8192 | map_map | 1.878340 | 1.916799 | -2.05% |
| 8192 | map_filter | 1.928729 | 1.969243 | -2.10% |
| 8192 | map_reduce | 1.357516 | 1.393542 | -2.65% |
| 8192 | record_map | 4.917438 | 4.996660 | -1.61% |
| 8192 | record_sort_key | 8.320615 | 8.443604 | -1.48% |
| 8192 | record_sort_compare | 96.653229 | 74.781771 | +22.63% |
| 8192 | reduce_growing_copy | 291.082876 | 285.706499 | +1.85% |

### Process-median ranges at 2,048 elements

These are four-process minima/maxima, not confidence intervals.

| Operation | Accepted step 2 range ms | Step 3 candidate range ms |
| --- | ---: | ---: |
| list_map | 0.217027–0.219649 | 0.218937–0.220859 |
| list_map_capture | 0.407438–0.411685 | 0.407487–0.413533 |
| list_filter | 0.337690–0.344658 | 0.342730–0.348918 |
| list_reduce | 0.206632–0.208298 | 0.212464–0.215277 |
| map_map | 0.480498–0.484272 | 0.482556–0.492639 |
| map_filter | 0.497485–0.503471 | 0.506390–0.517638 |
| map_reduce | 0.359497–0.361272 | 0.361926–0.369435 |
| record_map | 1.231972–1.250912 | 1.230597–1.260911 |
| record_sort_key | 2.072433–2.104508 | 2.084933–2.125717 |
| record_sort_compare | 19.981792–20.028167 | 15.183333–15.344916 |
| reduce_growing_copy | 18.116750–18.460208 | 18.088750–18.552333 |

### Separate CLI controls

Medians of 15 measured samples per variant after 3 warmups; these include startup,
parsing and setup and are separate from execution-only measurements above.

| Control | Accepted step 2 ms | Step 3 candidate ms |
| --- | ---: | ---: |
| Startup | 5.606708 | 5.618375 |
| numeric_loop | 31.645791 | 31.694750 |
| function_calls | 16.654208 | 16.616750 |
| list_pipeline | 65.602208 | 65.752250 |

### Interpretation

Record comparator sorting uses 21.9–24.0% less time across the three sizes.
The narrow reuse rule removes repeated structural traversal of the same small
records; argument snapshots, callback execution and result checks still cost
time. This does not remove collection copying or change growing-copy complexity.

Other callback medians were mostly slower. The largest increase was scalar
reduce at 8,192 elements, 3.87%; at 2,048 its process ranges did not overlap.
These helpers do not opt into argument holders, but the observation remains a
tradeoff of this candidate binary. Code layout/compiler effects are possible;
this experiment does not establish a cause or justify dismissing the increase
as noise. All four separate CLI medians changed by less than 0.3%.

The targeted sort improvement is substantial, but this is a candidate for user
review, not an accepted universal language speedup. Step 4 remains unstarted.

### Evidence

Locally retained ignored artifacts are under
`benchmarks/callbacks/evidence/argument-validation-2026-11-26/`: fresh
`before-change/`, the preserved initial `preview/`, and `final-paired/`.
The final directory contains eight raw engine reports, the shared fixture,
combined evidence, working-tree patch and source snapshots (including new tests).
The audit verified 2904 finite positive batch samples, 120 CLI observations,
alternating process order, medians, fixture/source/binary hashes, the accepted
step-2 baseline identity and pre-edit production hashes against HEAD.

| Frozen executable | SHA-256 |
| --- | --- |
| baseline-engine | 185d6759eb11b377858d218c7ea89a71a9c5884cda92418cf32174596e5683e7 |
| candidate-v2-engine | 4ce4f75e9115ccc4a1728701c2764d76c4cbca43eec00000a9277be22fb33744 |
| baseline-cli | 73babd81364c8bde3e3ba33f7cc48e47a36434b3d8b3ada740209353838f7072 |
| candidate-v2-cli | cbe8588224aceedeb526259bf30dea02f239a61b145b030c1d7cde749630b895 |

## Correctness coverage

Differential runtime tests compare the reusable path with ordinary owned
invocation, including callback writes to cloned records, nested reentrant calls,
instruction/scope/recursion limits, exact byte boundaries, throws, exits, source
locations, stack traces, failed assignment and recovery. Native payload tests
compare hook order and reference counts, including a payload that grows during
the callback. Invalid floats, duplicate map keys, excessive nesting, the 64-node
boundary, callables and a newly enabled byte budget retain fresh validation.
A compile-fail example checks the holder's immutable-borrow contract.

Collection tests compare the new input preparation against the original root
measurement across empty/scalar/nested inputs, limit boundaries and cancellation
triggered by a native payload observation. Existing collection tests cover sort
stability, custom comparators, callback failures and resource accounting.

## Validation environment

Native macOS 27.0.1 ARM64 build 26A434, pinned rustc 1.99.1
(58cf8f9c3, LLVM 23.1.1), explicit target aarch64-apple-darwin and
MACOSX_DEPLOYMENT_TARGET=15.0. Commands use --locked --offline.

- Focused runtime checks: 226 unit and 42 integration tests passed.
- Focused stdlib checks: 83 unit, 12 callback and 22 collection tests passed.
- Full workspace default features: 1230 passed, 38 ignored, no failures.
- Full workspace all features: 1229 passed, 45 ignored, no failures.
- Workspace Clippy, all targets/features, warnings denied: passed.
- Formatting, locked metadata and benchmark-driver syntax checks: passed.

Full tests used approved escalation for localhost/process fixtures; assertions
were unchanged. Ignored tests retain their existing platform/feature conditions.
No changed source examples require execution. Native macOS 15, Linux and Windows
baselines and dist/package validation remain pending; this local experiment does
not establish them. No heap profiling, compatibility update, commit or release.
