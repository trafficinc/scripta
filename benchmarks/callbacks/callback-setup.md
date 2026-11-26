# Callback setup experiment: step 2

Status: accepted by the user on 2026-11-26. Step 1 (owned arguments)
was accepted on 2026-11-26. This experiment measures incremental gains against
that accepted build; its results are not measured against the older PHP/Python/
Node reference or the pre-step-1 interpreter.

## Change and guards

The runtime already caches numeric function plans and reuses argument buffers
and empty scope records. This candidate avoids repeatedly activating two empty
scope records for eligible warmed numeric leaf calls, including callbacks. It
reuses the existing numeric plan without adding new compiled body forms.

Eligibility requires an existing cached plan, numeric values for every argument
(including unused ones), no receiver, primitive numeric or absent annotations,
and unset scope/value-byte quotas. Numeric plans cannot read captured variables,
call other code, create closures, mutate state or evaluate defaults/variadics.
Cold calls, general annotations, unsupported bodies/arguments and methods retain
ordinary setup. Configured scope or value-byte quotas use the original path.
A closure that retains a parent but reads only its numeric parameters can qualify;
a closure that reads its captured offset does not qualify.

Owner, arity and call-depth checks happen before this path; the active script
frame and defining source remain installed. Argument/return annotations and the
body checkpoint run in their existing order. The source and stack are restored
on success or failure. There are no scope/GC effects to reproduce inside an
eligible numeric expression, and configured quota paths preserve original scope
accounting. No value validation, cancellation or instruction check is removed.
The change stays inside scr-runtime and adds no public API or dependency.

## Reproduce

Preserve the accepted step-1 engine and CLI before rebuilding. Build the candidate
with the pinned native release configuration, following the commands in the
[callback comparison](README.md). The shared before/after driver now accepts
additional source paths to record the actual changed implementation:

```sh
python3 benchmarks/callbacks/owned_arguments.py --baseline BASELINE_ENGINE --candidate CANDIDATE_ENGINE --baseline-cli BASELINE_CLI --candidate-cli CANDIDATE_CLI --source crates/scr-runtime/src/lib.rs --source crates/scr-runtime/src/numeric_call.rs --source crates/scr-runtime/src/numeric_call/setup_tests.rs --output benchmarks/callbacks/evidence/callback-setup-local
```

Use separate frozen files for old and new executables. The driver refuses an
existing output directory. Four fresh process pairs alternate engine order;
identical fixtures cover 128, 2048 and 8192 elements. Each case warms for at least
100 ms and measures 11 calibrated batches (target 10 ms, maximum 10000 calls).
Every batch's full final result is checked outside timing. Tables summarize the
median of four process medians, with raw samples and ranges retained. Finish
builds/tests before measuring; no timing thresholds or forced GC are used.

Parsing, registration, callback declarations, input construction and output
validation are outside execution timing. Execution, argument handling, validation,
callbacks, assignments and normal cleanup are inside. Fixtures/callbacks are
unchanged from step 1. Additional controls check empty-process startup and the
existing numeric_loop, function_calls and list_pipeline CLI scripts, each with
3 warmups and 15 alternating measured samples; these include startup/parsing.
All command arguments, source/fixture/binary hashes, compiler/host and working-tree
patch are retained. Optional --source affects evidence only, not the timers.

## Results

Final candidate: **accepted by the user on 2026-11-26**. Step 1 is accepted;
step 3 is authorized; steps 4–5 and standard-library section 10 remain unstarted. Starting revision
`c165f68ecfd0385659f0134cdf98b73107bf3116` was clean before this experiment.

Native host: macOS 27.0.1 ARM64 build 26A434; rustc 1.99.1 (58cf8f9c3,
2026-10-13), LLVM 23.1.1, explicit `aarch64-apple-darwin` target. Both variants
use the same release profile, default SQLite CLI features and deployment setting
15.0. This does not establish native macOS 15 or other-platform validation.

The baseline engine/CLI hashes match the accepted step-1 final binaries. A fresh
pre-edit run was retained separately. The candidate engine is identical to the
successful preview engine; the CLI was rebuilt with the matching implementation.
Every output check in the final four-pair experiment passed.

### Execution observations

Median milliseconds per operation, summarized from four process medians. Positive
saved-time percentages are less elapsed time; negative values are slower. These
measurements are not universal speedups or statistical equivalence guarantees.

#### 128 elements

| Operation | Accepted step 1 ms | Step 2 candidate ms | Time saved |
| --- | ---: | ---: | ---: |
| Numeric list map | 0.039837 | 0.038013 | +4.58% |
| Captured-variable map (control) | 0.050860 | 0.050139 | +1.42% |
| Boolean list filter (control) | 0.046532 | 0.046096 | +0.94% |
| Scalar list reduce | 0.039568 | 0.037207 | +5.97% |
| Map map (mixed typed keys) | 0.056812 | 0.055006 | +3.18% |
| Map filter (control) | 0.057028 | 0.056274 | +1.32% |
| Map reduce (mixed typed keys) | 0.048246 | 0.046565 | +3.49% |
| Record map (control) | 0.102058 | 0.100156 | +1.86% |
| Record key sort (control) | 0.154018 | 0.152204 | +1.18% |
| Record comparator sort (control) | 0.764673 | 0.756475 | +1.07% |
| Growing-copy reduce (control) | 0.145292 | 0.143776 | +1.04% |

#### 2,048 elements

| Operation | Accepted step 1 ms | Step 2 candidate ms | Time saved |
| --- | ---: | ---: | ---: |
| Numeric list map | 0.244523 | 0.218228 | +10.75% |
| Captured-variable map (control) | 0.414851 | 0.408730 | +1.48% |
| Boolean list filter (control) | 0.346522 | 0.342581 | +1.14% |
| Scalar list reduce | 0.234665 | 0.207767 | +11.46% |
| Map map (mixed typed keys) | 0.506363 | 0.482430 | +4.73% |
| Map filter (control) | 0.507647 | 0.502086 | +1.10% |
| Map reduce (mixed typed keys) | 0.380600 | 0.359871 | +5.45% |
| Record map (control) | 1.253546 | 1.246049 | +0.60% |
| Record key sort (control) | 2.097796 | 2.088317 | +0.45% |
| Record comparator sort (control) | 20.126604 | 20.022479 | +0.52% |
| Growing-copy reduce (control) | 18.205709 | 18.075875 | +0.71% |

#### 8,192 elements

| Operation | Accepted step 1 ms | Step 2 candidate ms | Time saved |
| --- | ---: | ---: | ---: |
| Numeric list map | 0.883901 | 0.784079 | +11.29% |
| Captured-variable map (control) | 1.575384 | 1.544417 | +1.97% |
| Boolean list filter (control) | 1.293057 | 1.279036 | +1.08% |
| Scalar list reduce | 0.848564 | 0.750101 | +11.60% |
| Map map (mixed typed keys) | 1.960118 | 1.881590 | +4.01% |
| Map filter (control) | 1.942361 | 1.936514 | +0.30% |
| Map reduce (mixed typed keys) | 1.433443 | 1.360771 | +5.07% |
| Record map (control) | 4.988351 | 4.901292 | +1.75% |
| Record key sort (control) | 8.501489 | 8.458011 | +0.51% |
| Record comparator sort (control) | 98.260521 | 96.695771 | +1.59% |
| Growing-copy reduce (control) | 285.105021 | 291.090416 | -2.10% |

### Four-process median ranges at 2,048 elements

Minima/maxima below are process-median ranges, not confidence intervals.

| Operation | Accepted step 1 range ms | Step 2 candidate range ms |
| --- | ---: | ---: |
| Numeric list map | 0.241834–0.246650 | 0.217387–0.219010 |
| Captured-variable map (control) | 0.411577–0.419856 | 0.408230–0.410042 |
| Boolean list filter (control) | 0.342783–0.350052 | 0.338815–0.344146 |
| Scalar list reduce | 0.232277–0.236655 | 0.206810–0.208316 |
| Map map (mixed typed keys) | 0.500460–0.512858 | 0.480488–0.484458 |
| Map filter (control) | 0.500962–0.517238 | 0.500958–0.502767 |
| Map reduce (mixed typed keys) | 0.375293–0.384505 | 0.359205–0.360854 |
| Record map (control) | 1.240713–1.273833 | 1.232556–1.248167 |
| Record key sort (control) | 2.078200–2.154317 | 2.069458–2.096425 |
| Record comparator sort (control) | 20.043083–20.693500 | 19.951375–20.067750 |
| Growing-copy reduce (control) | 17.852708–18.726792 | 18.024625–18.394041 |

### Separate CLI observations

These include startup, lexing/parsing and setup; they are not execution-only
callback timings and are never subtracted from the table above.

| Control | Accepted step 1 ms | Step 2 candidate ms | Time saved |
| --- | ---: | ---: | ---: |
| Startup | 5.989208 | 5.902791 | +1.44% |
| numeric_loop | 33.883709 | 32.337250 | +4.56% |
| function_calls | 19.540458 | 16.797125 | +14.04% |
| list_pipeline | 67.566542 | 67.735959 | -0.25% |

### Interpretation and scope

At 2,048 elements numeric map used about 10.8% less time and scalar reduce about
11.5% less. At 8,192 elements the corresponding savings were 11.3% and 11.6%.
Map map/reduce benefit less because integer keys are eligible while numeric
string keys force the existing nonnumeric-argument path.

Captured-variable reads, boolean predicates and record callbacks did not enter
the new path. Their small timing changes should not be attributed to scope
elision. The 8,192-element growing-copy reduction recorded a 2.1% slower median;
its process ranges are retained below rather than claiming unchanged speed.
Growing-copy reduction remains quadratic and is not improved by this step.

The function-call CLI control used about 14% less time. The numeric-loop control
used about 4.6% less, but that workload does not establish a causal benefit from
this change; compiler layout and host/process variation also influence results.
The list-pipeline control recorded about 0.25% more time. These are specific
local observations, not overall language performance claims.

Configured scope/value quotas retain the old path, so this experiment does not
claim gains for quota-enabled embeddings or web workers. Native function bodies, general
user-function bodies and captured-variable evaluation are not accelerated by this
guard. Broader callback preparation, validation caching, new execution-plan body
forms and copy-on-write collections remain future work. No heap profiler was run.

Largest growing-copy reduction process medians (ms):

- Accepted step 1: 278.964167, 286.246666, 283.963375, 296.621292.
- Step 2 candidate: 290.786416, 291.394416, 290.372375, 292.262125.

### Correctness and checks

- Focused runtime: 221 unit tests, plus 42 function/invocation/resource/memory-
  lifecycle integration tests passed. The numeric-focused selection passed 33,
  including four new setup differential tests.
- New tests verify fewer physical scope entries in real reentrant callbacks,
  positive path hits, cold-call behavior, identical outputs/errors/source spans/
  stack traces/checkpoint counts, arithmetic/type failures and recovery, exact
  instruction/scope/value/call boundaries, and fallback for captures, defaults,
  variadics, receivers, general annotations and nonnumeric values.
- Existing native cancellation and captured-resource lifetime tests passed with
  the new path, alongside the broader runtime ownership and cleanup checks.
- Focused stdlib: 81 unit, 12 callback integration and 22 collection tests passed.
- Full default workspace: 1,222 passed, 0 failed, 38 existing ignored.
- Full all-features workspace: 1,221 passed, 0 failed, 45 existing ignored.
- All-workspace/all-target/all-feature Clippy with -D warnings passed, along with
  Cargo formatting and locked/offline metadata. All Cargo commands use the pinned
  native target and MACOSX_DEPLOYMENT_TARGET=15.0; full suites use approved
  escalation for existing localhost/process/filesystem fixtures.
- No source examples, language-visible APIs, dependency pins, lockfile entries or
  unsafe policy changed. No new compiled body forms or cached runtime Values.

Other native platform baselines, dist/package/extracted smoke and the overall
pre-expansion revision comparison remain pending. No compatibility changes,
commits, release or publication. Step 2 is accepted. Step 3 remains unstarted.

### Local evidence

Artifacts are locally ignored under
`benchmarks/callbacks/evidence/callback-setup-2026-11-26/`:

- `before-change/`: fresh accepted-baseline check and pre-edit source identities.
- `preview/`: preliminary matched correctness/performance observations.
- `final-paired/`: all eight reports, shared fixture, raw samples, hashes, compiler/
  host, extra changed-source identities and working-tree patch.
- `baseline-engine`, `baseline-cli`, `candidate-engine`, `candidate-cli`: frozen
  executable files, preserved independently of future Cargo builds.

Logs: `/tmp/scripta-setup-numeric-tests.log`,
`/tmp/scripta-setup-runtime-tests.log`, `/tmp/scripta-setup-stdlib-tests.log`,
`/tmp/scripta-setup-clippy.log`, `/tmp/scripta-setup-default-tests.log`,
`/tmp/scripta-setup-all-features-tests.log`,
`/tmp/scripta-setup-bench-build.log`, `/tmp/scripta-setup-cli-build.log`, and
`/tmp/scripta-setup-final-paired.log`.


Final audit passed: 2,904 positive finite batch observations, 120 CLI observations,
all source/fixture/frozen-binary hashes, aggregate medians and alternating order.
Accepted-baseline hashes match step 1, and pre-edit implementation hashes match
HEAD. Cargo formatting, locked/offline metadata, driver syntax, 37 local
documentation paths and final diff/whitespace checks passed.
