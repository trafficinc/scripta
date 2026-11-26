# Owned callback argument experiment: step 1

Status: ready for review; acceptance pending. This experiment covers
only argument ownership. Callback preparation, validation caching, wider function
execution plans and copy-on-write collections remain separate future steps.

## Change

InvocationContext::invoke_owned accepts a fixed-size owned Value array. It runs
the same checkpoint, callable and value validation as borrowed invoke, then moves
the arguments into the existing reusable buffer. It avoids a second deep clone
of temporary record/accumulator arguments and does not allocate a separate Vec
at the call site. Ordinary callable execution, stack traces, limits and cleanup
remain shared. The borrowed slice interface remains available.

Map::map/filter, List::reduce, Map::reduce and List::sortWith use this path.
List::map/filter and List::sortBy key extraction already pass borrowed values
without the redundant temporary copy; their invocation stays borrowed. Inputs
still receive the snapshots required by collection value semantics.

## Reproduce

Before changing runtime code, preserve the optimized benchmark executable and
CLI from the baseline build. Build the candidate with the same pinned compiler,
release profile, native target and feature configuration. On Apple Silicon:

```sh
MACOSX_DEPLOYMENT_TARGET=15.0 cargo bench -p scr-stdlib --bench callback_comparison --no-run --locked --offline --target aarch64-apple-darwin
MACOSX_DEPLOYMENT_TARGET=15.0 cargo build -p scr-cli --release --bin scripta --locked --offline --target aarch64-apple-darwin
python3 benchmarks/callbacks/owned_arguments.py --baseline BASELINE_ENGINE --candidate CANDIDATE_ENGINE --baseline-cli BASELINE_CLI --candidate-cli CANDIDATE_CLI --output benchmarks/callbacks/evidence/owned-local
```

ENGINE paths refer to the benchmark executables printed by Cargo. Baseline and
candidate must be separate frozen files; rebuilding in place would otherwise
replace the baseline. For a pre-edit baseline check, omit candidate/CLI paths
and use --rounds 1. Use a new output directory for each run. The evidence is
locally ignored and never overwritten.

Four rounds alternate baseline/candidate order, each in a fresh process with
identical fixtures. Every case warms for at least 100 ms, then runs 11 measured
batches at 128, 2048 and 8192 elements; calibration targets 10 ms per batch,
capped at 10000 operations. Full results are verified outside timing. Timing
includes execution, checks, copies and normal execution cleanup; parsing,
registration, fixture construction and result verification are outside. Median
results summarize four process medians, with their individual values retained.
No timing thresholds or forced GC are used. Finish tests/builds before timing.

Both programs run sequentially using the same release profile with overflow
checks, default Runtime options and unchanged benchmark source. Per-process
report files, fixture/source/binary hashes, compiler/host, working-tree patch,
raw samples and round order are retained. Other language results are not used
as a before/after baseline and are not mixed into candidate measurements.

Separate CLI checks alternate 3 warmups and 15 measured launches for startup,
numeric_loop, function_calls and list_pipeline fixtures from the established
comparison suite. Those controls include process startup, parsing and setup;
they are not execution-only callback timings. Every output is checked, including
warmups, with LF/CRLF normalization. Startup is never subtracted.

## Results

Final candidate: **ready for review, acceptance pending**. No later optimization
step or standard-library section 10 has started. Baseline is revision
`396a5626486509cb0235716f2846156ddbc58c53` with the already-added benchmark tooling. The first
pre-edit runtime/stdlib source hashes match that HEAD revision; baseline engine
and CLI hashes match the earlier four-language evidence. Both old and new
executables were frozen before running and are retained separately.

Host: macOS 27.0.1 ARM64 (26A434), native `aarch64-apple-darwin`, pinned
rustc 1.99.1 (58cf8f9c3, 2026-10-13), LLVM 23.1.1. Both builds use the same
release profile, default features and MACOSX_DEPLOYMENT_TARGET=15.0. This is not
native macOS 15 or other-platform validation. Every benchmark output check passed.

### Initial trial and revision

The first owned-argument candidate improved comparator sorting and growing-copy
reduction but introduced repeatable scalar overhead: at 2,048 elements, list map
took 6.3% more time, filter 5.6% more, and scalar reduce 4.7% more. This trial was
not presented for acceptance. Its results, source snapshots and frozen binaries
remain under `paired/` in the evidence folder.

That candidate factored checkpoint/value checks into a shared validation method.
The revised candidate adds an inlining hint to this method, retaining exactly the
same checks and ordering. A preview removed the observed scalar slowdown; the
full validation suites and final four-pair run below use this revised candidate.
No checks were removed and no additional optimization step was started.

### Matched execution timings

Milliseconds per operation, median of four process medians. Positive saved-time
percentages mean lower elapsed time; negative percentages are slower observations.
These are local workload measurements, not guaranteed speedups.

#### 128 elements

| Operation | Before ms | Candidate ms | Time saved |
| --- | ---: | ---: | ---: |
| List map (borrowed control) | 0.040262 | 0.039915 | +0.86% |
| Captured map (borrowed control) | 0.051182 | 0.050919 | +0.51% |
| List filter (borrowed control) | 0.046752 | 0.046692 | +0.13% |
| List reduce | 0.039702 | 0.039111 | +1.49% |
| Map map | 0.058053 | 0.056601 | +2.50% |
| Map filter | 0.058234 | 0.056799 | +2.47% |
| Map reduce | 0.050217 | 0.047957 | +4.50% |
| Record map (borrowed control) | 0.102060 | 0.101722 | +0.33% |
| Record key sort (borrowed control) | 0.155303 | 0.152739 | +1.65% |
| Record comparator sort | 0.946148 | 0.762718 | +19.39% |
| Growing-copy reduce | 0.177388 | 0.144604 | +18.48% |

#### 2,048 elements

| Operation | Before ms | Candidate ms | Time saved |
| --- | ---: | ---: | ---: |
| List map (borrowed control) | 0.252006 | 0.246557 | +2.16% |
| Captured map (borrowed control) | 0.424876 | 0.417068 | +1.84% |
| List filter (borrowed control) | 0.352429 | 0.350225 | +0.63% |
| List reduce | 0.251686 | 0.237202 | +5.75% |
| Map map | 0.540523 | 0.508432 | +5.94% |
| Map filter | 0.545913 | 0.511201 | +6.36% |
| Map reduce | 0.427404 | 0.383749 | +10.21% |
| Record map (borrowed control) | 1.300862 | 1.281424 | +1.49% |
| Record key sort (borrowed control) | 2.201946 | 2.148717 | +2.42% |
| Record comparator sort | 26.201334 | 20.265854 | +22.65% |
| Growing-copy reduce | 26.885084 | 18.585521 | +30.87% |

#### 8,192 elements

| Operation | Before ms | Candidate ms | Time saved |
| --- | ---: | ---: | ---: |
| List map (borrowed control) | 0.891401 | 0.898302 | -0.77% |
| Captured map (borrowed control) | 1.571518 | 1.581509 | -0.64% |
| List filter (borrowed control) | 1.291664 | 1.308125 | -1.27% |
| List reduce | 0.883730 | 0.866245 | +1.98% |
| Map map | 2.034946 | 1.985579 | +2.43% |
| Map filter | 2.012920 | 1.971267 | +2.07% |
| Map reduce | 1.559131 | 1.447671 | +7.15% |
| Record map (borrowed control) | 5.050653 | 5.140701 | -1.78% |
| Record key sort (borrowed control) | 8.591136 | 8.643073 | -0.60% |
| Record comparator sort | 122.722626 | 101.753791 | +17.09% |
| Growing-copy reduce | 396.357208 | 286.046271 | +27.83% |

### Process-median ranges at 2,048 elements

Ranges are minima/maxima of four process medians, not confidence intervals.
Warmup, heap reuse, CPU/OS state and untimed verification can influence timings.

| Operation | Before range ms | Candidate range ms |
| --- | ---: | ---: |
| List map (borrowed control) | 0.241187–0.261262 | 0.242548–0.258851 |
| Captured map (borrowed control) | 0.409217–0.427156 | 0.410235–0.440274 |
| List filter (borrowed control) | 0.338676–0.358112 | 0.338788–0.357892 |
| List reduce | 0.238775–0.254552 | 0.232362–0.244099 |
| Map map | 0.517025–0.543252 | 0.500329–0.533231 |
| Map filter | 0.515598–0.549748 | 0.505219–0.515406 |
| Map reduce | 0.404107–0.430630 | 0.377340–0.392514 |
| Record map (borrowed control) | 1.242065–1.327120 | 1.243755–1.315937 |
| Record key sort (borrowed control) | 2.092108–2.252658 | 2.107567–2.226325 |
| Record comparator sort | 24.690916–26.351500 | 20.114416–21.511125 |
| Growing-copy reduce | 25.432500–26.926000 | 18.228583–18.873167 |

### CLI controls

These are separate full-process milliseconds, median of 15 samples after three
warmups, with variant order alternating. They include startup/parsing/setup and
are not mixed with the execution-only table.

| Control | Before ms | Candidate ms | Time saved |
| --- | ---: | ---: | ---: |
| Startup | 6.121458 | 6.055834 | +1.07% |
| numeric_loop | 33.566958 | 33.230542 | +1.00% |
| function_calls | 19.598625 | 19.549625 | +0.25% |
| list_pipeline | 67.272625 | 67.473334 | -0.30% |

### Interpretation and acceptance scope

At 2,048 elements the revised candidate used about 23% less comparator-sort time
and 31% less growing-copy reduction time. At 8,192 elements the corresponding
savings were 17% and 28%. The quadratic growth risk in collection-building reduce
remains; loops remain preferable for that operation. Scalar map/filter results
do not demonstrate a broad speedup from this change.

Unchanged 8,192-element controls recorded roughly 0.6–1.8% slower medians. Their
per-process distributions overlap; these small differences remain observations,
not proof of either equivalence or a reproducible regression. The earlier
5–6% scalar slowdown did not recur after the validation inlining change. CLI
control medians moved by about -0.3% to +1.1% in saved time. No global performance
or memory-footprint guarantees follow from this run. No heap profiler was run;
the allocation-transfer regression test directly verifies absence of the extra
list payload copy, while the benchmarks measure complete operations.

The remaining callback-preparation, validation-walk, execution-plan and copy-on-
write proposals are unimplemented. The cross-language table remains a separate
pre-optimization reference; PHP/Python/Node were not rerun for this candidate.

### Correctness and tooling checks

- Final focused runtime invocation: 7 passed. Broader focused runtime functions,
  invocation, resources and memory lifecycle: 42 passed before the inlining hint.
- Final callback integration: 12 passed, including owned reducer/map closure
  retention across explicit GC, nested snapshots, resource cleanup after failure,
  typed keys, stable comparators and unchanged destinations. Existing collection
  behavior (22) and stdlib unit tests (81) also passed.
- Final default workspace suite: 1,218 passed, 0 failed, 38 existing ignored.
- Final all-features workspace suite: 1,217 passed, 0 failed, 45 existing ignored.
- Full workspace/all-target/all-feature Clippy with warnings denied passed after
  the inlining revision; final Cargo formatting and locked/offline metadata passed.
- New public-path tests cover 0/1/2/3 owned arguments, actual storage transfer,
  borrowed copy preservation, exact/adjacent byte ceilings, non-finite values,
  duplicate keys, depth, callable ownership, arity, reentrancy, source/trace, typed
  throws, exit, instruction/scope/recursion limits and cancellation.
- Cargo tests/Clippy/builds use --locked --offline and explicit native target,
  with MACOSX_DEPLOYMENT_TARGET=15.0. Full suites use approved escalation for
  existing localhost/process/filesystem fixtures. Assertions were not weakened.
- No dependencies, lockfile pins, parser behavior or script API exports changed.
  This adds only the public native owned-invocation method and its stdlib uses.

Other native baselines (macOS 15 ARM64, Ubuntu 22.04 x86-64/ARM64 and Windows
Server 2022 x86-64), dist-profile/package evidence and the expansion-wide
old/new-revision comparison remain pending. No compatibility claims, commits,
publication or release.

### Evidence

All experiment artifacts are locally ignored under
`benchmarks/callbacks/evidence/owned-arguments-2026-11-26/`:

- `before-change/`: fresh pre-edit baseline check and original source hashes.
- `paired/`: first four-pair trial, archived sources, and its scalar regression.
- `inline-preview/`: preliminary check of the validation inlining revision.
- `final-paired/`: final four-pair measurements, CLI controls, complete outputs
  checked by each engine, raw samples, hashes, compiler/host and working-tree patch.
- `baseline-engine`, `baseline-cli`, initial `candidate-engine`/`candidate-cli`,
  and final `candidate-v2-engine`/`candidate-v2-cli` are frozen binaries.

Logs: `/tmp/scripta-owned-final-default-tests.log`,
`/tmp/scripta-owned-final-all-features-tests.log`,
`/tmp/scripta-owned-clippy.log`,
`/tmp/scripta-owned-final-invocation-tests.log`,
`/tmp/scripta-owned-final-callback-tests.log`,
`/tmp/scripta-owned-v2-bench-build.log`,
`/tmp/scripta-owned-v2-cli-build.log`, and
`/tmp/scripta-owned-final-paired.log`.


Final audit passed: 2,904 positive finite measured batch observations, 120 CLI
startup/control observations, source/fixture/frozen-binary hashes, aggregate
medians and alternating order. All unchanged large-case process ranges overlap.
Cargo formatting, locked/offline metadata, driver syntax, 43 local documentation
paths and final diff/whitespace checks passed.
