# Temporary performance improvement plan

Working checklist for improving Scripta's competitiveness with Python and PHP.
This is a temporary implementation plan, not a public feature roadmap. Creating
this document does not start implementation or approve every step in advance.

## Execution and approval

Work through the numbered steps in order. The three targeted optimizations from
the original plan have separate checkpoints so each can be reviewed on its own.

- Start a step only after the user explicitly asks to start it or approves the
  proposed next step. An "OK" following a next-step summary approves that step.
- Complete the authorized work, relevant tests, measurements, and documentation
  before presenting its result. Resolve failures within that scope before review.
- After each step, update this document with the result and checks actually run,
  summarize the next step, and stop for the user's OK. Do not automatically
  continue through the remaining checklist.
- If profiling changes the recommended order or scope, explain the evidence and
  propose the revised next step before starting it.
- Keep unrelated working-tree edits intact. Do not commit, publish, change release
  tooling, or update platform-support claims as part of this plan.

Current checkpoint: **Step 8 ready for review. Sequence complete; bytecode removed, AST retained.**

## Shared requirements

Follow [repository guidance](../AGENTS.md), the
[language specification](language-spec.md), and the
[native API contracts](standard-library.md). Preserve strict types, checked
arithmetic, Unicode semantics, collection value semantics, evaluation order,
failed-assignment behavior, error kinds, source spans, traces, cancellation,
execution budgets, resource ownership, and cleanup. Keep safe Rust and the
declared minimum Rust version. Keep standard-library backends out of the runtime.

Keep the existing [comparison workloads](../benchmarks/comparison/README.md)
unchanged when measuring improvements. Use compatible optimized builds on the
same idle native host, preserve baseline/candidate binary identities, and save
new raw results rather than replacing evidence. Separate execution-only timings
from whole-process CLI timings. Report observed changes and variability without
promising parity with another language.

For every implementation step, run focused behavior and boundary tests first.
For cross-crate, public API, dependency, or feature changes, run the full checks
required by AGENTS.md, including default/all-feature coverage where applicable.
Use explicit native targets and the pinned toolchain; set the required deployment
target for macOS. Cover exact limits and limit-plus-one, cancellation, cleanup,
and unchanged destinations after failure where relevant. Record unavailable
native-platform validation as pending rather than treating local tests as proof.

## 1. Profile and preserve a baseline

Status: **Ready for review.**

- [x] Preserve the optimized baseline binary and verified comparison results.
- [x] Add or extend execution-only measurements alongside the unchanged CLI suite.
- [x] Profile numeric loops, recursive Fibonacci, list pipelines, concatenation,
      dictionaries, methods, regex, JSON, and function calls.
- [x] Attribute CPU time and allocations to parsing, evaluation, name lookup,
      call setup, copying, validation, collection, and native-library work.
- [x] Record the leading measured costs and confirm or revise the next priority.

Completion evidence: reproducible commands, binary/source identities, checked
outputs, raw measurements, profiler findings, and relevant harness tests.

### Completion record

Authorization: the user's "ok" after creation of this checklist.

Changed: added an optimized parsed-program measurement worker, a checked
execution-only driver, a separate native CPU/allocation capture driver, harness
regressions, and reproduction instructions. No production evaluator or library
behavior was changed. The existing comparison fixtures and published tables are
unchanged.

The CLI baseline is preserved at
`target/performance-baselines/step1/scripta`; its SHA-256 matches the retained
CLI comparison evidence. A symbolized optimized worker is preserved beside it as
`comparison_profile`, with `manifest.json` and `source-hashes.json`. These ignored
local artifacts must remain available for later comparisons; a Cargo clean would
remove them. All benchmark measurements ran separately from builds and tests.

Measurements: 15 observations after 3 warmups for every workload. Times below
are median milliseconds from the parsed-program worker, not a replacement for
whole-process PHP/Python/Scripta results. The worker uses fresh runtimes, no
attached source text, and no CLI module resolver; its build includes optimized
code and debug information for profiling.

| Workload | Lex | Parse | Setup | Execute | Teardown |
| --- | ---: | ---: | ---: | ---: | ---: |
| numeric_loop | 0.015 | 0.014 | 0.299 | 432.574 | 0.051 |
| recursive_fib | 0.011 | 0.007 | 0.258 | 458.883 | 0.032 |
| list_pipeline | 0.019 | 0.016 | 0.328 | 179.531 | 0.042 |
| string_concat | 0.011 | 0.008 | 0.254 | 1015.856 | 0.031 |
| dict_ops | 0.020 | 0.012 | 0.233 | 82.556 | 0.030 |
| class_dispatch | 0.016 | 0.012 | 0.258 | 267.296 | 0.037 |
| regex_match | 0.012 | 0.008 | 0.225 | 304.513 | 0.029 |
| json_roundtrip | 0.015 | 0.010 | 0.196 | 73.131 | 0.029 |
| function_calls | 0.012 | 0.008 | 0.271 | 141.656 | 0.034 |
| startup | 0.000 | 0.000 | 0.311 | 0.091 | 0.051 |

CPU captures use separate three-second sampling windows during repeated checked
executions. Observed leading costs:

- Regex: 73.3% of main-thread samples include `RegexBuilder::build`.
- Concatenation: 92.6% of sampled self time is in byte copying.
- Methods: at least 30.3% of sampled self time is in native allocator functions;
  method binding still clones declarations and allocates a callable per access.
- Numeric loop: at least 27.2% of sampled self time appears in hashing or
  environment-name lookup. Fibonacci and plain calls also show lookup, call-frame,
  scope allocation/release, and evaluator work.
- Dictionary operations show indexed reads/writes and key comparisons prominently.
  List pipelines show evaluator, hashing, scope, and allocation work. JSON shows
  substantial allocation activity. Parsing is tiny for these particular fixtures.
- Collection and validation remain visible but are not established as the leading
  bottleneck in these captures. This is not evidence to remove correctness checks.

Self-symbol summaries omit entries with fewer than five samples, so grouped self
percentages are lower bounds. The inclusive regex percentage overlaps with
allocator/copying samples and must not be added to them.

Allocation logging captured all ten workloads separately from timing. Recorded
concatenation allocation volume was 111.76 GB over the run, with 502,268 allocation
events; regex recorded 5,487,260 allocations and 860.9 MB cumulative volume.
These are instrumented lifetime heap/VM log totals, including launch, parsing,
setup, and logging overhead. Compacted logs may omit immediate allocation/free
pairs. They are not exact Rust allocation counts, resident memory, or memory
leaks. Native high-water stacks sometimes reflect profiler initialization, so
live stacks and CPU allocation activity were retained alongside them.

Evidence:

- [Execution observations](../benchmarks/comparison/evidence/step1-execution.json).
- [Profile summary and raw-capture hashes](../benchmarks/comparison/evidence/step1-profile-summary.json).
- Raw CPU captures: `target/performance-baselines/step1/cpu/`.
- Raw peak/live allocation captures and logs:
  `target/performance-baselines/step1/alloc/`.
- The CPU driver's source at capture time is archived locally as
  `target/performance-baselines/step1/native_profile_cpu.py`; the later change
  selects full allocation logging for allocation captures only.
- [Measurement and profiling commands](../benchmarks/comparison/README.md#execution-only-profiling).

Checks and limitations: the optimized worker's three regressions passed with
both default and minimal features. Python comparison/profiling tests passed
(13 tests). Focused default/all-feature Clippy with warnings denied, Rust
formatting, document links, evidence/source hashes, and `git diff --check` passed.
Focused Rust checks used `--release --locked --offline` with explicit native
target `aarch64-apple-darwin`, deployment target 15.0, and the same debug-information
setting as the worker build. The full workspace suite was not rerun because
this step only adds benchmark tooling and does not alter production behavior,
public APIs, dependencies, or feature wiring. Only this local native host was
measured; Linux and Windows performance/profiling remain unvalidated.

The sandbox initially blocked process sampling and CPU metadata; approved runs
outside it succeeded. Allocation capture initially used the host's default lite
logging mode, which lacks high-water history; retrying with documented full mode
succeeded. Failed attempts remain marked failed in separate local directories.

Recommendation: retain the planned order. Regex caching is the next isolated
change, supported by a dominant measured compilation cost. Method binding and
string buffer reuse remain the following separate steps; no optimization has
been implemented yet.

User acceptance / authorization to continue: **Step 2 explicitly requested by the user.**

Next-step summary: add a bounded runtime-local regex cache so repeated API calls
reuse compiled patterns while preserving limits and error behavior.

## 2. Cache compiled regex patterns

Status: **Ready for review.**

- [x] Define bounded cache entry/storage policies and eviction before coding.
- [x] Reuse successful compilations across the registered regex APIs within a
      runtime, using the public native registration interface.
- [x] Preserve pattern/compiled-size limits, Unicode matching, cancellation,
      error categories, output budgets, and runtime teardown behavior.
- [x] Test repeated patterns, eviction, different patterns, invalid patterns,
      limit boundaries, and isolation between runtimes.
- [x] Measure regex improvements and cache memory costs; update affected API docs.

### Completion record

Authorization: the user's explicit request to implement Step 2.

Changed: `scr-stdlib` registration closures share an LRU cache across
`Regex::isMatch`, `Regex::findAll`, and `Regex::replace`. Exact pattern text is
the key, including inline flags. Only successful compilations are retained.
Policy chosen before implementation: at most 8 entries and 256 KiB of pattern
text, evicting the least recently used entry until both bounds hold. The existing
65,536-byte pattern ceiling and per-regex 1 MiB compiled/DFA ceilings remain.
Fixed entry limits bound backend retention independently of script value limits;
these engine limits are not exact heap accounting. Metadata/scratch add overhead,
and a successful miss briefly holds one extra compiled regex before eviction.
Invalid patterns leave existing entries unchanged. No dependencies, runtime API,
exports, or script signatures changed. The API and architecture references now
describe cache ownership, bounds, cancellation, and teardown.

Regression coverage: reuse and LRU promotion, exact entry/key-byte ceilings and
one-over eviction, distinct patterns, invalid/unsupported/compiled-size failures,
exact pattern bytes and one over, sharing across the three APIs, independent
runtimes, and teardown of both cache and compiled values. Integration tests cover
warm-cache output ceilings (including exact replacement bytes and one over),
unchanged assignments after errors, repeated error locations/traces, and
cancellation followed by successful reuse. Existing Unicode, empty-pattern,
literal replacement, and output-budget tests still pass.

Measurements: unchanged 20,000-call regex fixture, optimized native worker,
3 warmups per binary, then 15 seeded interleaved observations using fresh
runtimes. Baseline execute median **315.739 ms** (311.818–323.613); candidate
**16.603 ms** (16.160–17.619), about **19.0× faster** on this workload.
These are parsed-program execution times, not whole-process CLI timings; the
published PHP/Python/CLI tables remain unchanged. Native allocation capture ran
separately: recorded allocations **5,487,258 → 132,548**, recorded cumulative
heap/VM volume **860.9 MB → 8 MB**. Rounded current allocated heap/VM grew
**317 KB → 331 KB**; live stack totals grew **168K → 184K**, consistent
with retaining the compiled pattern and scratch state. Rounded high-water totals
were **1 MB** for both (units as printed by the profiler). These single captures include launch/profiler overhead,
and compacted logs can omit immediate allocation/free pairs. They do not measure
maximum cache occupancy or establish an exact heap bound.

Raw observations, binary/source hashes, native capture manifests and hashes,
host/compiler information, and validation receipts are retained in
[`step2-regex.json`](../benchmarks/comparison/evidence/step2-regex.json).
Ignored local binaries (`baseline`, `candidate`), allocation stacks, test logs,
and the timing reproduction script (`measure.py`) remain under
`target/performance-baselines/step2/`. The baseline was rebuilt from the unchanged
pre-Step-2 implementation before editing; both workers use the same build command
recorded in the evidence. Run the reproduction script with a fresh evidence
filename; it refuses to overwrite results. Allocation reproduction uses
`benchmarks/comparison/native_profile.py --worker WORKER --kind alloc
--workload regex_match --directory NEW_DIRECTORY`.

Validation at base revision `4c962c9e939b7f381a770955f42deba25f33f21b` plus
these working-tree changes, Rust 1.99.1, native `aarch64-apple-darwin`, macOS
27.0.1, `MACOSX_DEPLOYMENT_TARGET=15.0`:

- Focused `scr-stdlib --lib regex_cache`: 3 passed.
- Focused `scr-stdlib --test data_handling regex`: 5 passed.
- Full workspace tests with default features and with all features: passed.
  Initial sandbox runs were blocked by localhost socket restrictions; both full
  configurations passed outside the sandbox with approval, without assertion
  changes. Full suites include collection behavior/resource tests.
- Workspace all-target/all-feature Clippy with warnings denied: passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- Cargo build/test/Clippy commands used `--locked --offline` and explicit target
  `aarch64-apple-darwin`. No dependency pins changed.

Linux x86-64/ARM64, Windows x86-64, and the macOS 15 native baseline remain
pending; this host is macOS 27.0.1. No release-support claims were changed.

Next-step summary: reduce direct method-call allocation and declaration cloning
while keeping bound methods, inheritance, receivers, and traces correct.

## 3. Reduce method-dispatch overhead

Status: **Ready for review.**

- [x] Share immutable method declarations instead of cloning their bodies when
      binding a method.
- [x] Optimize direct method calls to avoid constructing temporary bound callable
      objects where observable behavior permits.
- [x] Preserve escaped method values, receiver identity, lexical ownership,
      inheritance/interfaces, exceptions, and retained source locations.
- [x] Test direct and escaped calls, callable identity where observable,
      cross-runtime ownership, collector roots, cancellation, and cleanup.
- [x] Measure dispatch time, allocations, and collector activity.

### Completion record

Authorization: the user's "ok" following the Step 2 completion and proposed
method-dispatch step.

Changed: runtime class metadata owns immutable `Rc<FunctionDeclaration>` values;
bound methods share method bodies instead of cloning them on every access.
Standalone user declarations also use the shared declaration representation.
Each escaped method read still creates a distinct callable identity with the
original receiver, lexical environment, runtime owner, and definition source.

Eligible direct property calls resolve a script method before argument evaluation
and invoke the common call-frame implementation without allocating or tracking a
temporary bound callable. Grouped/escaped method values, callable fields, and
native properties keep their value-call semantics. Native property lookup and
optional downcasts still occur once. Constructors continue through ordinary
bound-callable invocation. The direct path retains the receiver as an external
object root and explicitly roots the declaring method's lexical environment
across argument evaluation and entry. Nested roots unwind on failure, cancellation,
and exit. No checkpoints were added or removed. Public AST/native/value APIs,
exports, dependencies, and syntax remain unchanged. The architecture reference
describes declaration sharing, dispatch, and temporary roots.

Regression coverage includes direct, grouped, and escaped calls; callable identity
and copies; defaults and spreads; receiver mutation/capture; interfaces; callable
fields; native getters and effectful receiver/argument order; missing/non-callable
members; arity failures; cross-runtime objects and bound methods; retained source
locations and traces; recursion/scope limits; identical instruction-budget
outcomes; exact logical value limits and one over; and cancellation in arguments,
defaults, and method bodies with unchanged destinations and runtime recovery.
Existing exception inheritance, module/interface identity, cycle/resource cleanup,
function, and memory-lifecycle suites still pass. A targeted collector test removes
all global object/class roots and collects during nested inherited-method
arguments, proving that the differing lexical owner survives and temporary roots
are released on both success and argument failure.

Measurements: unchanged comparison fixtures, optimized native parsed-program
worker, 3 warmups per binary/workload, then 15 seeded interleaved observations
using fresh runtimes. Builds, tests, allocation captures, and timing ran separately.

| Workload | Baseline execute median (range), ms | Candidate execute median (range), ms |
| --- | ---: | ---: |
| class_dispatch | 271.698 (264.779–280.625) | 213.406 (210.589–231.488) |
| function_calls (control) | 142.672 (140.407–174.225) | 143.768 (141.419–145.398) |

The 200,000-call class workload takes **21.5% less execution time** in this local
measurement. The plain-function control median is **0.8% higher**, with overlapping
sample ranges; these samples do not establish a meaningful control improvement
or regression. These are execution-only measurements, not whole-process CLI
results; published comparison tables remain unchanged.

Separate native allocation capture for class dispatch recorded **4,015,626 →
1,602,342 allocations** (60.1% fewer) and **291.2 MB → 96.7 MB** cumulative
heap/VM volume. Rounded current allocated heap/VM was **331 KB → 318 KB**;
live stack totals were **184K → 173K**. Rounded high-water totals were **1 MB**
for both (units as printed by the profiler). These single captures include
launch/profiler overhead, compacted logs may omit immediate allocation/free pairs,
and heap/VM totals are not process RSS.

Deterministic collector counters on the candidate's 20,000-call test reported
**2 collections** for direct calls (execute boundaries only) versus **80** when
explicitly grouped method accesses force bound-callable creation. Both produce
the same result and retain only global/object scopes with no temporary roots or
user-function registry entries afterward. This compares the two candidate paths;
it is not an instrumented historical-baseline counter.

Raw observations, binary/source hashes, allocation manifests and stack hashes,
collector observations, host/compiler information, and validation receipts are in
[`step3-methods.json`](../benchmarks/comparison/evidence/step3-methods.json).
The preserved baseline is the Step 2 candidate; its identity matches the Step 2
evidence. Ignored binaries, raw allocation stacks, test/lint logs, and timing
reproduction script (`measure.py`) remain in `target/performance-baselines/step3/`.
The script refuses to overwrite evidence; choose a fresh output filename for a
repeat. Native allocation reproduction uses
`benchmarks/comparison/native_profile.py --worker WORKER --kind alloc
--workload class_dispatch --directory NEW_DIRECTORY`.

Validation at base revision `4c962c9e939b7f381a770955f42deba25f33f21b` plus
working-tree changes, pinned Rust 1.99.1, native `aarch64-apple-darwin`, macOS
27.0.1, `MACOSX_DEPLOYMENT_TARGET=15.0`:

- Focused runtime method tests: 2 internal tests and 8 integration tests passed.
- Full `scr-runtime` tests and CLI `language_extensions`, `interfaces`, and
  `exceptions` integration targets: passed.
- Full workspace tests with default features and with all features: passed.
  Both ran outside the sandbox with approval for the existing localhost fixtures.
  They include collection behavior/resource tests.
- Workspace all-target/all-feature Clippy with warnings denied: passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- Cargo build/test/Clippy commands used `--locked --offline` and explicit target
  `aarch64-apple-darwin`. No dependency pins changed.

Linux x86-64/ARM64, Windows x86-64, and native macOS 15 validation remain pending.
This host runs macOS 27.0.1; no release-support claims were changed.

Next-step summary: extend safe buffer reuse from `.=` to eligible
`$s = $s . suffix` assignments without altering evaluation or failure behavior.

## 4. Reuse string buffers for eligible assignments

Status: **Ready for review.**

- [x] Identify exactly which assignment forms can safely reuse the destination.
- [x] Extend the existing append optimization to eligible self-concatenation.
- [x] Retain the general evaluation path for side effects, aliases, or expression
      contexts that cannot safely use the optimized path.
- [x] Test Unicode, empty strings, self-reference, effectful expressions, used
      assignment results, exact budgets, allocation failures where testable,
      cancellation, and destination preservation.
- [x] Measure the unchanged concatenation workload and allocation/copy costs.

### Completion record

Authorization: the user's "ok" following Step 3 and the proposed string-buffer
reuse step.

Changed: the existing discarded `.=` buffer-reuse path also accepts discarded
`$s = $s . suffix` assignments to an existing string variable. Eligibility was
chosen before implementation: the concat prefix must be exactly the destination
variable (grouping is allowed), and the suffix must pass the existing purity
check. The suffix is evaluated into an owned value before reservation/mutation;
self-reference therefore sees the old string and independent copies stay
independent. Checked logical-byte growth and fallible reservation precede append.
The `=` path retains concat-expression error spans and error categories, while
`.=` retains its existing target spans and overflow behavior. No checkpoints
were added or removed.

Calls, property reads, effectful suffixes, different prefix variables, prepends,
left-associated concat chains, indexed/property destinations, and used assignment
results retain the general path. This avoids changing left-value snapshots,
intermediate limits, or expression result ownership. No public APIs, syntax,
exports, dependencies, or logical resource policies changed. The architecture
reference now states exact eligibility and fallback rules.

Eight integration regressions cover actual buffer-capacity reuse, Unicode and
empty strings, self-reference, independent string/collection copies, captured
bindings, used/chained assignment results, nonmatching prefixes, indexed and
property targets, native property getter counts, effectful suffixes that mutate
the destination, and preservation of prior effects on failure. Differential tests
force the existing general assignment path while retaining identical source/spans,
then compare full errors and destinations for type/index/arithmetic/undefined-name
failures and exact UTF-8 byte ceilings plus one-over failures. Intermediate concat
limits and definition-source traces are covered. Instruction exhaustion during
iterative pure-suffix work and cancellation before assignment or in an effectful
suffix preserve the destination and allow runtime recovery.

Real allocator exhaustion was not injected: there is no existing allocator fault
hook, and introducing unsafe allocation hooks or risking host exhaustion was not
part of this step. The existing fallible reservation call still precedes mutation;
its failures propagate without committing an append. Capacity reuse and logical
budget failures are exercised deterministically.

Measurements: unchanged fixtures, same optimized native parsed-program worker,
3 warmups per binary/workload followed by 15 seeded interleaved observations using
fresh runtimes. Builds, tests, timing, allocation capture, and CPU sampling ran
separately.

| Workload | Baseline execute median (range), ms | Candidate execute median (range), ms |
| --- | ---: | ---: |
| string_concat | 1020.879 (986.248–1083.668) | 37.448 (37.004–38.124) |
| function_calls (control) | 143.046 (141.768–200.818) | 142.732 (141.032–148.478) |

The 100,000-append string workload is about **27.3× faster** in this local
execution-only measurement. The plain-function control median is **0.2% lower**,
with overlapping sample ranges; these samples do not establish a meaningful
control improvement. These are not whole-process CLI timings, and the published
PHP/Python/Scripta comparison tables remain unchanged.

Separate native allocation capture recorded **502,269 → 202,303 allocations**
(59.7% fewer) and **111.76 GB → 6 MB** cumulative heap/VM allocation volume.
Rounded current allocated heap/VM was **1.8 MB → 1.3 MB** and high-water totals
were **3.3 MB → 2.1 MB** (units as printed by the profiler). Cumulative volume
is not simultaneous live memory; heap/VM totals are not RSS. Captures include
launch/profiling overhead, and compacted logs can omit immediate allocation/free
pairs.

Separate 3-second CPU samples during repeated fresh-runtime executions attributed
**1,800 / 1,953 (92.2%) → 43 / 2,320 (1.9%)** self samples to reported
`memmove`/`memcpy` symbols. These percentages are lower bounds from a summary
that omits symbols with fewer than five self samples; they are time attribution,
not exact copied-byte totals. The eliminated prefix clones account for
**39,999,600,000 bytes** by derivation from `sum(8 * i for i in range(100000))`.
That derived quantity excludes suffix construction, geometric buffer growth,
end-of-fixture correctness checks, and other copies.

Raw observations, baseline/candidate source and binary hashes, native CPU and
allocation manifests/stack hashes, compiler/host details, and validation receipts
are in [`step4-strings.json`](../benchmarks/comparison/evidence/step4-strings.json).
The preserved baseline is the Step 3 candidate, verified against its evidence.
Ignored binaries, the baseline runtime source snapshot, native stacks, test/lint
logs, and timing reproduction script (`measure.py`) remain under
`target/performance-baselines/step4/`. The script refuses to overwrite evidence;
choose a fresh output filename for a repeat. Native captures reproduce with
`benchmarks/comparison/native_profile.py --worker WORKER --kind alloc`
(or `--kind cpu`) `--workload string_concat --directory NEW_DIRECTORY`.

Validation at base revision `4c962c9e939b7f381a770955f42deba25f33f21b` plus
working-tree changes, pinned Rust 1.99.1, native `aarch64-apple-darwin`, macOS
27.0.1, `MACOSX_DEPLOYMENT_TARGET=15.0`:

- Focused `scr-runtime --test resources`: 9 passed.
- New `scr-runtime --test string_assignment` regressions: 8 passed in the full
  runtime and workspace suites.
- Full `scr-runtime` tests: passed.
- Full workspace tests with default features and with all features: passed.
  Both ran outside the sandbox with approval for existing localhost fixtures;
  they include collection behavior/resource tests.
- Workspace all-target/all-feature Clippy with warnings denied: passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- Cargo build/test/Clippy commands used `--locked --offline` and explicit target
  `aarch64-apple-darwin`. No dependency pins changed.

Linux x86-64/ARM64, Windows x86-64, and native macOS 15 validation remain pending.
This host runs macOS 27.0.1; no release-support claims were changed.

Next-step summary: reduce identifier lookup and call-frame overhead, introducing
indexed locals only where bindings can be resolved without changing semantics.

## 5. Reduce variable and function-call overhead

Status: **Ready for review.**

- [x] Use profiling to select identifier interning, fewer repeated lookups,
      reusable call storage, and/or indexed local bindings.
- [x] Preserve dynamic includes, lexical assignment/shadowing, closures, module
      aliases, recursion, default arguments, and host-injected globals.
- [x] Define invalidation or fallback behavior for bindings that remain dynamic.
- [x] Test scope boundaries, retained captures, reentrant native calls, stack
      traces, recursion/step/scope limits, and failed calls.
- [x] Measure loops, Fibonacci, plain calls, and methods, plus memory behavior.

### Completion record

Authorization: the user's "ok" following Step 4 and the proposed lookup/call-frame
optimization step.

Selection: the existing CPU profiles attribute at least 27.2% of numeric-loop self
samples to hashing/environment lookup and 11.6% to allocation. Fibonacci and plain
calls also show lookup, scope, and allocation costs. This step selects fewer
repeated name lookups, borrowed assignment names, and reusable lexical-scope
storage. Indexed locals and identifier interning are not introduced; dynamic
bindings continue through the live lexical maps.

Changed: successful variable/function lookup returns its scope and value together,
avoiding a second hash lookup to read the value. Assignment places borrow variable
and property names from their live AST, and existing variable entries are replaced
in place. New bindings still allocate their owned map key. Assignment targets
remain resolved before RHS evaluation, and transactional indexed writes still
validate the complete candidate before mutation.

Empty scope storage is reused through a best-effort runtime-local pool, bounded
to **64 scopes**, each with a sum of at most **32 map/set capacity slots**
(**2,048 slots** across the pool). All values, keys, function/class/alias bindings,
module paths, parents, capture flags, and function-boundary flags are cleared
before pooling. Captured scopes recycle only after tracing establishes that they
are unreachable. Oversized scopes and pool-reservation failures use ordinary
scope destruction. The pool contains no graph roots, does not count toward live
scope limits, and is freed with its runtime. Its capacity bounds are not an exact
heap/RSS ceiling. The architecture reference documents this ownership and policy.

Dynamic behavior: every resolution still reads the current lexical maps; no
binding-index cache, miss cache, or invalidation protocol is needed. Includes,
shadowing, closures, isolated-module native fallback, late host registration,
and host-injected globals observe current bindings. Reused storage carries no
old names or namespace state. No checkpoints, public AST/native/value APIs,
syntax, exports, dependencies, or feature wiring changed.

Regression coverage: three new internal tests cover pool bounds and oversized
maps, clearing all five binding/module namespaces and lifetime flags, release
of callable/class ownership, and reentrant global-map growth with prior mutations
preserved on failure. Five runtime integration tests cover global isolation after
frame reuse, scope shadowing, independent captures across transient-call churn,
host injection/new registration after failed lookups, reentrant lexical updates,
and exact live-scope ceilings plus one-over failure and recovery. Two CLI tests
cover dynamic includes/captured defaults, repeated use of the same local alias for
different modules, explicit LF/CRLF byte fixtures, isolated native fallback, and
exclusion of caller script functions. Existing defaults, recursion/step limits,
cancellation, errors/traces, retained-source, resource cleanup, and memory-lifecycle
suites also pass.

Measurements: unchanged fixtures, the same optimized native parsed-program worker,
3 warmups per binary/workload followed by 15 seeded interleaved observations using
fresh runtimes. Builds, tests, timing, and allocation captures ran separately.

| Workload | Baseline execute median (range), ms | Candidate execute median (range), ms | Median reduction |
| --- | ---: | ---: | ---: |
| numeric_loop | 435.025 (433.724–454.949) | 399.058 (396.462–429.537) | 8.3% |
| recursive_fib | 471.833 (468.210–475.695) | 425.765 (423.797–453.038) | 9.8% |
| function_calls | 145.755 (143.283–177.230) | 134.209 (132.500–171.447) | 7.9% |
| class_dispatch | 213.777 (208.466–219.887) | 199.534 (197.162–232.104) | 6.7% |

These are local execution-only observations, not whole-process CLI timings or
speedup guarantees. Plain-call and method ranges overlap. The published
PHP/Python/Scripta tables remain unchanged.

Separate native allocation captures:

| Workload | Recorded allocations, baseline → candidate | Cumulative heap/VM volume | Current allocated heap/VM |
| --- | ---: | ---: | ---: |
| numeric_loop | 2,002,203 → 2,200 | 7.7 MB → 2 MB | 305 KB → 306 KB |
| recursive_fib | 4,116,085 → 3,087,659 | 363 MB → 131.6 MB | 326 KB → 349 KB |
| function_calls | 1,202,246 → 602,244 | 75.3 MB → 29.1 MB | 308 KB → 310 KB |
| class_dispatch | 1,602,344 → 802,341 | 96.7 MB → 49.6 MB | 318 KB → 319 KB |

The pool trades a small amount of retained empty storage (rounded current totals
increase 1–23 KB) for fewer allocations: about 99.9% fewer recorded events in the
numeric loop, 25.0% in Fibonacci, and 49.9% in plain calls/methods. Rounded
high-water totals were **1 MB** for both binaries on all four workloads (units as
printed by the profiler). These single captures include launch/profiling overhead;
compacted logs may omit immediate allocation/free pairs, so events are not exact
Rust allocation counts. Cumulative volume is not simultaneous live storage, and
heap/VM totals are not RSS. No new CPU sampling result is claimed for this step;
the earlier CPU evidence informed selection, and these new timings/allocation
captures measure the resulting change.

Raw timings, baseline/candidate source and binary hashes, allocation manifests and
stack hashes, pool policy, test-source hashes, compiler/host details, and validation
receipts are in [`step5-bindings.json`](../benchmarks/comparison/evidence/step5-bindings.json).
The baseline is the preserved Step 4 candidate, verified against its evidence.
Ignored binaries, baseline runtime/environment source snapshots, native stacks,
test/lint logs, and timing reproduction script (`measure.py`) remain under
`target/performance-baselines/step5/`. The script refuses to overwrite evidence;
choose a fresh output filename for a repeat. Allocation captures reproduce with
`benchmarks/comparison/native_profile.py --worker WORKER --kind alloc`
plus `--workload` for the four measured fixture names and
`--directory NEW_DIRECTORY`.

Validation at base revision `4c962c9e939b7f381a770955f42deba25f33f21b` plus
working-tree changes, pinned Rust 1.99.1, native `aarch64-apple-darwin`, macOS
27.0.1, `MACOSX_DEPLOYMENT_TARGET=15.0`:

- Focused pool tests: 2 passed; all 3 new internal tests passed in the full runtime
  and workspace suites.
- Runtime `binding_storage`: 5 passed. CLI `binding_storage`: 2 passed.
- Full `scr-runtime` tests: passed.
- Full workspace tests with default features and with all features: passed.
  Both ran outside the sandbox with approval for existing localhost fixtures;
  they include collection behavior/resource tests.
- Workspace all-target/all-feature Clippy with warnings denied: passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- Cargo build/test/Clippy commands used `--locked --offline` and explicit target
  `aarch64-apple-darwin`. No dependency pins changed.

Linux x86-64/ARM64, Windows x86-64, and native macOS 15 validation remain pending.
This host runs macOS 27.0.1; no release-support claims were changed.

Next-step summary: investigate collection copying and ordered-map lookup, using
profiles to choose improvements compatible with the native Value interface.

## 6. Improve measured collection hot paths

Status: **Ready for review.**

- [x] Locate unnecessary copies and repeated validation in list/map operations.
- [x] Evaluate indexed ordered-map lookup while preserving insertion order,
      strict equality, and distinct integer/string keys.
- [x] Preserve independent nested collection values, ownership, logical-byte
      accounting, cancellation, and failed-update behavior.
- [x] Avoid changing the public Value representation in this step. If a measured
      improvement requires that change, present its compatibility implications
      as a separate proposal for user approval.
- [x] Test copy independence, order, typed keys, native/callable identities,
      resource boundaries, and cleanup. Measure small and growing collections.

### Completion record

Authorization: the user's "ok" following Step 5 and the proposed collection
optimization step.

Selection: dictionary CPU profiles identify core `index_ref`, `set_index`, and
key comparisons prominently. Inspection found temporary string-key copies on
core lookup/update, linear ordered-map search, and repeated validation passes at
public native/host boundaries. Those boundary checks remain necessary. Existing
native `Map::merge`/key-set helper indexes already use borrowed typed keys and
remain unchanged.

Changed: core indexing compares borrowed typed keys, owning a new key only when
inserting a new entry. Variable-rooted map paths may use bounded runtime-local
position hints for their first map segment. Each hint hit is verified against the
actual current vector key; misses fall back to ordinary search and never establish
absence. Whole-variable writes, host `set_global`, and destructuring invalidate
matching hints. Verification/fallback also handles reordered maps and recycled
scope IDs. Values are read from the current vector, and the indexes retain no
native/callable owners or graph roots.

Policy chosen before implementation: maps with **64–4,096 entries** are eligible;
retain at most **8 indexes**, **4,096 indexed positions in total**, and **256 KiB**
of UTF-8 string-key plus variable-name bytes. Reads build indexes lazily; writes
only reuse existing hints so insertion-only growth does not rebuild repeatedly.
Small/oversized maps bypass cache borrowing/build machinery. Allocation failure
declines the optimization. A newly built bounded index can briefly coexist with
the cache before eviction; these capacity/text limits are not an exact heap/RSS
cap. Nested map segments, property roots, and native helper lookups keep ordinary
lookup behavior. The architecture reference documents ownership, bounds, and
fallbacks.

The public `Value::Map(Vec<(MapKey, Value)>)` and owned `Value::List` representations
are unchanged. Native arguments/results keep independent nested collection copies;
native objects and callables retain their existing shared handle identities.
Insertion order, strict integer/string key distinctions, map equality, full logical
accounting/validation, checkpoints, error spans/kinds/traces, cancellation, and
transactional failed-update behavior remain. No public APIs, exports, dependencies,
feature wiring, or language syntax changed.

An exploratory cursor-based validation prototype matched the previous ledger in
differential tests but slowed the list pipeline (three-sample exploration:
170.936 → 188.674 ms). It was excluded. Final `value.rs` is byte-identical to the
Step 5 baseline. The prototype binary/source and exploration remain in the ignored
local evidence directory; no reduced-validation fast path is included.

Regression coverage: four internal tests cover typed positions, stale/reordered
hints and scope-ID reuse, LRU and total-position bounds, exact UTF-8 key-byte
ceiling and one over, small/oversized fallback, duplicate-map rejection for index
building, and release of callable ownership. Five runtime integration tests cover
nested copy independence, insertion order, typed keys, native/callable identities,
host replacement/destructuring/growth, reused parameter scopes, precise missing-key
traces, exact byte ceilings and one-over insertion failures, unchanged destinations,
malformed host-map rejection, cancellation/recovery, runtime isolation, and resource
cleanup. One standard-library integration test composes warmed script indexing
with native remove/merge, order, typed keys, and independent nested values.

Measurements: unchanged dictionary/list comparison workloads, numeric/plain-call
controls, and supplemental ordered-map/list size probes. Map probes include
construction/growth, 4,096 string-key read/update/read iterations, and final count
and sum checks. List probes repeatedly materialize slices and check total lengths.
Compare binaries within a probe row; work differs by size. Each complete run uses
3 warmups per binary/workload and 15 seeded interleaved observations with fresh
runtimes. Builds, tests, timing, and allocation profiling ran separately.

The first complete run had wide main-workload timing swings. It is retained at
[`step6-collections.json`](../benchmarks/comparison/evidence/step6-collections.json).
A second complete run used identical binaries, fixtures, seed, and sample counts;
the steadier repeat below is recorded in
[`step6-collections-repeat.json`](../benchmarks/comparison/evidence/step6-collections-repeat.json).
Residual outliers and overlapping ranges remain visible; these are local
observations rather than speedup guarantees.

| Workload | Baseline execute median (range), ms | Candidate execute median (range), ms |
| --- | ---: | ---: |
| dict_ops | 79.333 (78.304–112.593) | 53.493 (52.778–84.563) |
| list_pipeline | 162.001 (160.334–167.156) | 163.224 (161.339–188.665) |
| numeric_loop (control) | 393.627 (388.658–417.465) | 398.779 (392.970–425.884) |
| function_calls (control) | 131.362 (130.292–139.515) | 131.642 (130.476–134.134) |

Dictionary median execution time is **32.6% lower**. List/control medians are
**0.2–1.3% higher**, with overlapping ranges; no improvement is claimed for them.
These are parsed-program execution times, not whole-process CLI times. Published
PHP/Python/Scripta comparison tables remain unchanged.

| Supplemental probe | Baseline median, ms | Candidate median, ms |
| --- | ---: | ---: |
| map, 16 entries | 5.202 | 5.184 |
| map, 63 entries | 6.318 | 6.162 |
| map, 64 entries | 6.320 | 5.411 |
| map, 65 entries | 6.351 | 5.397 |
| map, 256 entries | 8.204 | 5.563 |
| map, 1,024 entries | 20.370 | 7.081 |
| map, 4,096 entries | 72.298 | 21.725 |
| map, 4,097 entries (fallback) | 74.457 | 61.933 |
| list, empty | 0.916 | 0.920 |
| list, 16 entries | 1.390 | 1.403 |
| list, 2,048 entries | 7.074 | 7.094 |
| list, 65,536 entries | 11.805 | 11.826 |

The 4,096-entry map probe is about **3.3× faster**, including its growing
construction phase. The 4,097-entry probe uses the linear fallback with borrowed
keys; no index beyond the configured limit is built.

Separate native allocation captures (units as printed by the profiler):

| Workload | Recorded allocations, baseline → candidate | Cumulative heap/VM volume | Current allocated heap/VM |
| --- | ---: | ---: | ---: |
| dict_ops | 465,256 → 465,517 | 23.7 MB → 23.7 MB | 344 KB → 345 KB |
| list_pipeline | 11,829 → 11,827 | 88.4 MB → 88.4 MB | 384 KB → 384 KB |
| map, 16 entries | 43,549 → 43,550 | 3.7 MB → 3.7 MB | 322 KB → 322 KB |
| map, 4,096 entries | 80,280 → 84,379 | 5.9 MB → 6.2 MB | 564 KB → 847 KB |

The largest probe retains about **283 KB** more allocated storage for its index;
its rounded high-water total increases **1 MB → 1.2 MB**. Other high-water totals
are rounded to 1 MB. These captures do not establish an allocation-count reduction:
index construction adds owned keys/positions, and compacted logs can omit immediate
allocation/free pairs such as transient lookup copies. Counts are recorded events,
not exact Rust allocation counts. Captures include launch/profiling overhead;
cumulative volume is not simultaneous live storage, and heap/VM totals are not RSS.
No new CPU-sampling result is claimed for this step.

The repeat report contains all timing observations, source/binary/test-source
hashes, bounds, allocation manifests and stack hashes, prototype exclusion,
compiler/host information, and validation receipts. The baseline is the preserved
Step 5 candidate, verified against its evidence. Ignored binaries, baseline source
snapshots, exploratory runs/prototypes, generated probe sources, native stacks,
test/lint logs, and reproduction scripts remain under
`target/performance-baselines/step6/`. `measure-repeat.py` reproduces timing; choose
a fresh output filename since it refuses to overwrite evidence. The original
comparison allocation driver reproduces dictionary/list captures; the retained
`profile-small-large.py` wrapper calls that same capture helper for the hashed
16-/4,096-entry probes. Profiling requires native macOS inspection access.

Validation at base revision `4c962c9e939b7f381a770955f42deba25f33f21b` plus
working-tree changes, pinned Rust 1.99.1, native `aarch64-apple-darwin`, macOS
27.0.1, `MACOSX_DEPLOYMENT_TARGET=15.0`:

- Final focused runtime checks: 47 unit tests and 5 `collection_hot_paths` tests
  passed; full runtime tests also passed.
- Focused collection resource checks: 6 passed. Collection integration target:
  22 passed, including the new native/script composition regression.
- Full workspace tests with default features and with all features: passed.
  Both ran outside the sandbox with approval for existing localhost fixtures.
- Workspace all-target/all-feature Clippy with warnings denied: passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- Cargo build/test/Clippy commands used `--locked --offline` and explicit target
  `aarch64-apple-darwin`. No dependency pins changed.

Linux x86-64/ARM64, Windows x86-64, and native macOS 15 validation remain pending.
This host runs macOS 27.0.1; no release-support claims were changed.

Next-step summary: design an internal bytecode engine with indexed locals and
explicit call frames, then establish semantic parity with the AST evaluator.

## 7. Introduce and validate an internal bytecode engine

Status: **Rejected by the user; removed during Step 8.**

- [x] Define instructions, value/frame ownership, source mapping, dynamic-binding
      fallbacks, collector roots, and semantic checkpoint accounting.
- [x] Compile parsed programs to internal instructions with indexed locals and
      explicit frames. Keep the AST evaluator available for comparison.
- [x] Cover supported language/native behavior, including closures, classes,
      modules/includes, callbacks, exceptions, and cleanup, through documented
      AST compatibility bridges where lowering is incomplete.
- [x] Compare outputs, globals, errors, spans, traces, limits, cancellation,
      resource lifetimes, and failed destinations.
- [x] Run existing behavior/resource suites with both engine selections and add
      compiler/VM differential regressions.
- [x] Measure entry compilation, execution, worker startup, allocation events,
      and native heap/VM storage. Preserve the normal AST default.

### Historical evaluation record

Step 8 methodology correction: the retained Step 6 worker was a plain optimized
release build, while the newer Step 7 worker included debug information. Small
cross-build differences against Step 6 are therefore not clean attribution to a
runtime change. The Step 7 AST/VM selections within the same newer binary remain
matched controls; the raw observations are preserved unchanged. Step 8 uses
plain-release CLI binaries for its paired timing and matching debug-information
settings for native allocation captures. Rebuilding the restored plain worker
produces the **identical SHA-256** as the accepted Step 6 binary.

The following records the tested Step 7 candidate, which is no longer present in
active source. Its APIs, feature, tests, and worker were removed during Step 8;
measurements and validation receipts are preserved as historical evidence.

Authorization: the user's "ok" following Step 6 and its proposed internal-engine
step. This authorizes an experimental implementation and readiness review, not a
silent production default switch or Step 8.

Changed: added an internal instruction compiler and hybrid VM with indexed
canonical binding storage, operand/target stacks, lexical/loop unwind records,
and explicit activations for lowered ordinary user calls. Root code is reusable
through `CompiledProgram`; shared function/method definitions lazily cache body
code. Shared call entry/exit preserves the existing arity/default/variadic/type,
source, trace, recursion, and value-validation rules for both interpreters.
Instructions retain original spans and the existing statement/loop checkpoint
schedule. Logical value and scope ceilings, cancellation, resource ownership,
and transactional failed writes remain enforced. The public Value representation,
standard-library exports, dependency pins, and source syntax are unchanged.

Normal builds still start with `Engine::Ast`. Hosts can explicitly select
`Engine::Bytecode`; `execute_compiled` temporarily selects it and restores the
previous engine. Its source-aware counterpart reapplies each runtime's source
ceiling. The explicit runtime Cargo feature `experimental-bytecode` selects the
VM initially, so workspace **all-feature validation now exercises the VM** while
default-feature validation exercises AST. This feature is experimental, not a
production recommendation. There is no new CLI flag or script environment switch.

Compatibility/ownership: only current-scope value slots are cached; all bridges,
calls, and scope changes invalidate hints. Ancestor and dynamic lookup retain
existing rules. Slots own values once, are visited by the collector, and are not
mirrors of hash-map values. Suspended operands, old assignment snapshots, and
callee handles remain ordinary owned graph roots. Compiled instructions own
syntax, not environments or runtime Values. Indexed layouts are released when
empty scopes return to the bounded storage pool, and its capacity accounting
includes binding maps and value vectors.

This is **not fully lowered bytecode**. Call arguments/defaults, collections and
indexing, direct property/method and static calls, string/optimized concatenation
assignment, declarations/classes, foreach/comprehensions, includes, try/catch/
finally, throw, and exit use existing AST operations, which may invoke compiled
bodies. Reentrant callbacks and some nested work can still use host recursion;
the existing hard call-depth ceiling of 64 remains. Compiler storage is outside
logical value budgets, and compilation is synchronous like parsing. Entry chunk
counts do not describe nested work inside bridges or lazy function chunks.
These limitations are documented in the architecture and embedding references.

Regression coverage: eight differential/API tests cover closures and recursive
calls, loop control through try/finally, classes and escaped methods, strict
short circuit/evaluation order, defaults/variadics, native callbacks, full
RuntimeError equality, unchanged destinations, every cutoff in three 0–89
instruction-budget matrices, scope/recursion/value boundaries, native-triggered
cancellation and recovery, reusable source-aware code and exact source ceilings,
engine restoration/switching, LF/CRLF includes and module errors, and resource
roots surviving collector checkpoints while other calls run. Existing ownership,
cleanup, output, permission, collection, language, CLI, web, and native tests run
with both default selections. No assertions were weakened or platform guards
removed.

Measurements: all ten original fixtures, 3 warmups per engine/workload, then
15 seeded interleaved observations per engine using fresh runtimes. The preserved
baseline hash matches the accepted Step 6 candidate. `AST` is the current code;
`VM` includes entry compilation in execution; `precompiled VM` measures entry
compilation separately. Lazy function-body compilation remains inside execution
in both VM modes. Builds, tests, timing, and allocation captures ran separately.

Execution medians, milliseconds:

| Workload | Step 6 baseline | Current AST | VM | Precompiled VM |
| --- | ---: | ---: | ---: | ---: |
| numeric_loop | 395.554 | 378.319 | 372.447 | 372.181 |
| recursive_fib | 419.249 | 436.060 | 523.094 | 522.056 |
| list_pipeline | 164.350 | 168.187 | 225.483 | 225.850 |
| string_concat | 36.063 | 34.296 | 35.495 | 35.466 |
| dict_ops | 53.731 | 56.639 | 54.498 | 54.417 |
| class_dispatch | 194.350 | 197.011 | 213.805 | 213.911 |
| regex_match | 16.115 | 15.525 | 15.505 | 15.376 |
| json_roundtrip | 71.801 | 71.366 | 74.394 | 74.114 |
| function_calls | 131.530 | 131.675 | 144.504 | 143.603 |
| startup | 0.045 | 0.043 | 0.046 | 0.044 |

Compared with the Step 6 baseline, precompiled VM numeric-loop median is **5.9%
lower**, but recursive Fibonacci is **24.5% higher**, list pipeline **37.4%
higher**, method dispatch **10.1% higher**, and plain calls **9.2% higher**.
Compared with the current AST engine, numeric-loop median is only **1.6% lower**;
this distinction matters when attributing improvements to VM dispatch.
The current AST control itself has mixed results: numeric loop **4.4% lower**,
string concatenation **4.9% lower**, recursive Fibonacci **4.0% higher**, and
dictionary operations **5.4% higher** than baseline. Dictionary ranges do not
overlap in this run (53.171–55.054 versus 56.228–57.945 ms), so that default-path
regression requires investigation before integration approval. No claim of
unchanged default-engine performance is made. All observations and ranges,
including outliers, are retained in
[`step7-bytecode.json`](../benchmarks/comparison/evidence/step7-bytecode.json).

Entry compile medians range from **1.833 µs** for the empty program to
**22.333 µs** for the list fixture; these are not whole-program ahead-of-time
compile times. Empty-program execution medians are 0.043–0.046 ms across current
engines. Complete worker-process medians for startup are 5.361 / 5.292 / 5.309 /
5.317 ms for baseline / AST / VM / precompiled VM. Those wall times include worker
launch, fixture reading/checking, parsing, setup, execution, and output checking;
they are not production CLI startup timings. Published PHP/Python/Scripta tables
remain unchanged for Step 8.

Separate native allocation captures, in baseline / current AST / precompiled VM
order (units as printed by the profiler):

| Probe | Recorded allocations | Cumulative allocated heap + VM | Current allocated heap + VM |
| --- | ---: | --- | --- |
| recursive_fib | 3,087,658 / 3,087,660 / 5,780,256 | 131.6 / 131.6 / 331.5 MB | 349 / 358 / 366 KB |
| function_calls | 602,246 / 602,246 / 1,002,325 | 29.1 / 29.1 / 56.6 MB | 310 / 311 / 328 KB |
| startup | 2,076 / 2,078 / 2,079 | 2 / 2 / 2 MB | 277 / 277 / 277 KB |

The VM adds substantial allocation churn, not a lower allocation count. Per-call
operand/slot-frame storage and indexed scope reuse are candidates for further
work, alongside bridge lowering; this step does not claim new CPU-sampling
attribution. All high-water totals round to 1 MB and their retained stacks are
dominated by profiler startup, so they do not establish equal workload peaks.
Recorded events can omit compacted immediate alloc/free pairs. Instrumented
heap/VM totals include launch/profiler overhead, are not RSS, and cumulative volume
is not simultaneous live storage. The allocation report and raw stack hashes are
in [`step7-allocations.json`](../benchmarks/comparison/evidence/step7-allocations.json).

Validation at base revision `4c962c9e939b7f381a770955f42deba25f33f21b` plus these
working-tree changes, pinned Rust 1.99.1, native `aarch64-apple-darwin`, macOS
27.0.1, `MACOSX_DEPLOYMENT_TARGET=15.0`:

- Focused differential target: **8 passed**, explicitly comparing both engines.
- Full workspace/default-feature suite: **774 passed** (normal AST selection).
- Full workspace/all-feature suite: **774 passed** (explicit VM selection).
  These suites ran outside the sandbox with approval for existing localhost
  fixtures. A preliminary VM run caught grouped self-concatenation losing buffer
  reuse; the fallback was corrected and the existing regression now passes.
- Focused collection resource checks: **6 passed**; collection integration:
  **22 passed**.
- Workspace all-target/all-feature Clippy with warnings denied: **passed**.
- Benchmark harness tests: **15 passed**, including malformed timing/count
  rejection; CPU metadata access required sandbox approval.
- Formatting, locked/offline metadata, and diff-whitespace checks: **passed**.
- Cargo checks/builds used `--locked --offline` and the explicit native target.
  No dependencies or lockfile pins changed. Optimized worker output/completion
  was validated on every timing/profile invocation.

Raw timing/allocation evidence, source/binary/fixture/driver hashes, and a linked
validation summary are retained in
[`step7-review.json`](../benchmarks/comparison/evidence/step7-review.json).
Ignored baseline/candidate binaries, pre-change runtime sources, logs, native
stacks, expected outputs, and validation receipts remain under
`target/performance-baselines/step7/`. The tracked comparison README documents
reproduction. No files belonging to unrelated work were reverted or committed.
Linux x86-64/ARM64, Windows x86-64, and native macOS 15 validation remain pending;
this local host does not establish release support.

**Readiness recommendation: keep the bytecode engine experimental and retain AST
as the normal default.** Behavioral parity passes the available suites, but
incomplete lowering, extra allocation churn, VM execution regressions, and the
measured default-AST dictionary regression prevent a default-ready recommendation.
These remain explicit review/integration work, not accepted performance gains.

Next-step summary: review the accumulated changes and remaining regressions,
make the default-engine decision explicitly, complete integration validation,
and refresh the published comparison tables. **Step 8 requires the user's OK.**

User decision / authorization: "ok then get rid of the experimental bytecode
engine if it is not working in all ways we need, and proceed to step 8".
The engine is rejected rather than retained as an opt-in feature. Step 8 removes
its common-path changes as well as the VM itself and validates the accepted AST
implementation before refreshing published results.

## 8. Final integration review and benchmark refresh

Status: **Ready for review. Optimization sequence complete; AST only.**

- [x] Confirm accepted implementations and remove the rejected experiment,
      including its shared AST-path changes.
- [x] Retain AST as the sole engine; preserve the accepted native API and Value
      representation. Remove all experimental APIs, features, tests, and workers.
- [x] Run full workspace/default/all-feature validation and harness checks;
      report unavailable native platforms as pending.
- [x] Collect a fresh complete PHP/Python/Scripta run without concurrent builds,
      tests, or profiling, retaining samples, environment, and binary hashes.
- [x] Generate README/performance tables from that same evidence using the
      existing generic presentation; update architecture/methodology references.
- [x] Record improvements, regressions, memory tradeoffs, remaining gaps,
      documentation links, and whitespace checks.

### Completion record

Authorization/default decision: the user explicitly requested removal of the
experimental engine and continuation to Step 8. **The bytecode candidate is
rejected and removed, not retained behind an opt-in feature.** AST is the sole
execution engine in every supported feature configuration. Final result acceptance
remains the user's review; authorization to work is not acceptance of measurements.

Changed: verified the Step 7 source hashes before restoring the four affected
runtime files to accepted Step 6 content. Removed `bytecode.rs`, its indexed
binding wrapper, VM integration tests, bytecode profiling example and Python
comparison/test drivers, the experimental Cargo feature, engine selection APIs,
and reusable compiled-program APIs. Active architecture, API, development, and
benchmark references describe the retained AST runtime. Historical Step 7 JSON
receipts remain unchanged; rejected source/tests/drivers are archived under
ignored `target/performance-baselines/step7/removed-source/` with hashes. No
unrelated working-tree edits were reverted; no commit, publication, tag, workflow,
or release operation was performed. The temporary plan is retained for review.

Accepted implementation: bounded runtime-local regex caching; direct-method
resolution with shared declarations; safe discarded string/self-concatenation
buffer reuse; lexical lookup and bounded empty-scope reuse; borrowed typed-map
keys and verified bounded map-position hints. No public Value representation,
strict types, UTF-8/Unicode rules, native signatures, cancellation/checkpoint
schedule, resource ceilings, failed-assignment semantics, or error locations/
traces were changed for integration. All **14 runtime source files** match the
accepted Step 6 hashes; regex source also matches its accepted hash.

The final plain-release execution worker has SHA-256
`2098fd1d59fec57aa988bab7954e6d58bdf494cbe69771d185351a9f00c95122`,
**identical to the accepted Step 6 worker**. This verifies removal of experimental
common-path changes as well as VM dispatch. Matched-control medians were 53.266 /
54.202 ms for two copies of that identical binary; their spread is not a code
regression. The rejected AST reference was 56.628 ms but uses the different debug
information profile and is historical context, not a clean attribution control.
See [`step8-dictionary-control-matched.json`](../benchmarks/comparison/evidence/step8-dictionary-control-matched.json).
The initial mismatched auxiliary check is also retained as
`step8-dictionary-control.json`; neither supplies published language tables.

Methodology correction: Step 7 mixed a plain-release retained baseline with a
newer release worker including debug information. Its within-new-worker AST/VM
comparisons remain matched, but small baseline-relative differences should not be
attributed solely to runtime edits. Step 8's published/paired CLI timings use
plain release; both native allocation workers use matching symbolized release
settings. This correction does not change the bytecode rejection or the fresh
CLI results. Actual build logs and hashes are retained rather than replacing old
evidence or claiming identical timing for identical binaries.

Validation: base revision `4c962c9e939b7f381a770955f42deba25f33f21b` plus the
working tree; pinned Rust 1.99.1, native `aarch64-apple-darwin`, macOS 27.0.1,
`MACOSX_DEPLOYMENT_TARGET=15.0`:

- Focused runtime suite: **156 passed**. Regex cache: **3 passed**; collection
  resource checks: **6 passed**; collection integration: **22 passed**.
- Full workspace tests with default features: **766 passed**. Full workspace
  tests with all features: **766 passed**. Both exercise AST after removal; the
  eight prototype tests account for the decrease from Step 7. Existing localhost
  fixtures ran outside the sandbox with approval; assertions were not weakened.
- Workspace all-target/all-feature Clippy with warnings denied: **passed**.
- Minimal CLI `cargo check --no-default-features`: **passed**. This is a compile
  check, not a claim of a separate minimal-feature test or release-package run.
- Benchmark harness tests: **13 passed**, including wrong output/exit, timeout,
  incomplete/corrupt evidence, dual-document generation, and byte-exact LF/CRLF
  preservation. CPU metadata access required sandbox approval.
- All 10 unchanged Scripta fixtures passed `check` and `fmt --check`; benchmark
  runs then executed and validated output. Syntax checks were not used as runtime
  proof. Cargo metadata/lockfile consistency, formatting, links, generated table
  consistency, binary/source hashes, and diff-whitespace checks passed.
- Build/test/check/Clippy commands used `--locked --offline` and explicit native
  target. The normal optimized CLI includes default SQLite. No dependency pins
  changed, and no engine migration is required for the accepted native API.

Published results: a fresh complete **30-row** PHP/Python/Scripta matrix with
15 measured samples after 3 warmups per workload/runtime, seed 1729. Every output
was validated: 450 measured invocations plus 90 warmups. Both README and
`docs/performance.md` are generated from the single immutable
[`macos-arm64-20261122-step8.json`](../benchmarks/comparison/evidence/macos-arm64-20261122-step8.json)
report. Exact runtime versions, host details, binary/fixture hashes, and every
sample remain in the raw report; presentation stays generic. No builds, tests,
other timing runs, or allocation profiling overlapped these measurements.

A separate seed-88 paired CLI run interleaves the preserved original Scripta
binary and final AST binary on the same idle host, with 3 warmups and 15 measured
observations per binary/workload. Its medians below are **not substituted** for
Scripta times in the independently collected three-language table. Raw samples,
order, source hashes, original baseline manifest, and the exact driver source
are in [`step8-before-after.json`](../benchmarks/comparison/evidence/step8-before-after.json).

| Workload | Original CLI median, ms | Final AST CLI median, ms | Median change |
| --- | ---: | ---: | ---: |
| numeric_loop | 425.373 | 403.930 | -5.0% |
| recursive_fib | 465.147 | 436.119 | -6.2% |
| list_pipeline | 181.996 | 169.843 | -6.7% |
| string_concat | 1046.879 | 42.854 | -95.9% |
| dict_ops | 87.287 | 59.338 | -32.0% |
| class_dispatch | 271.484 | 205.477 | -24.3% |
| regex_match | 314.978 | 21.106 | -93.3% |
| json_roundtrip | 80.143 | 76.966 | -4.0% |
| function_calls | 146.269 | 139.497 | -4.6% |
| startup | 5.299 | 5.281 | -0.3% |

Concatenation and regex have the largest observed improvements, with dictionary
and method work also materially lower. Startup ranges overlap, so no startup
improvement is claimed. Smaller changes are local observations, not guarantees.
The paired run retains substantial outliers (including a 381.891 ms final string
sample versus its 42.854 ms median); no sample was discarded or replaced.

Remaining PHP/Python gaps are explicit in the fresh table: arithmetic loops,
recursion, lists/dictionaries, and method/function calls still have higher Scripta
medians than both comparison commands. Scripta has lower regex/startup medians;
concatenation is close to Python, while JSON lies between PHP and Python. These
whole-process/API observations include startup and native-library behavior and
are not claims of a universally faster language or isolated regex-engine speed.

Separate matching-profile native allocation captures compare the preserved
original worker with the final AST worker, in baseline → final order:

| Workload | Recorded allocation events | Cumulative allocated heap + VM | Current allocated heap + VM | High-water allocated heap + VM |
| --- | ---: | --- | --- | --- |
| string_concat | 502,269 → 2,300 | 111.76 GB → 5.5 MB | 1.8 MB → 1.3 MB | 3.3 MB → 2.1 MB |
| dict_ops | 711,788 → 465,517 | 33.7 MB → 23.7 MB | 353 KB → 345 KB | 1 MB → 1 MB |
| regex_match | 5,487,261 → 82,545 | 860.9 MB → 7.9 MB | 317 KB → 323 KB | 1 MB → 1 MB |

Raw capture manifests and peak/live stack hashes are retained in
[`step8-memory.json`](../benchmarks/comparison/evidence/step8-memory.json).
Recorded events are not exact Rust allocation counts; compacted logs can omit
immediate pairs. Cumulative volume is not simultaneous live storage; allocated
heap/VM totals include launch and instrumentation and are not RSS. Rounded 1 MB
high-water stacks are dominated by profiler startup. These data do not establish
an aggregate heap cap or maximum cache occupancy.

Memory tradeoffs remain: regex retains up to 8 compiled patterns and 256 KiB of
pattern text, with the existing per-pattern/engine limits and scratch overhead.
Map hints retain up to 8 indexes, 4,096 positions total, and 256 KiB of key/name
bytes; the earlier 4,096-entry probe retained about 283 KB more index storage.
The current small dictionary capture does not prove that index storage is free.
Both caches can briefly hold a newly built candidate before eviction. The scope
pool retains at most 64 empty scopes with at most 32 aggregate capacity slots per
scope, clearing values and graph edges. Logical value limits remain distinct
from retained capacity and process memory; no public owned-collection semantics
were traded away for these gains.

The [integration evidence](../benchmarks/comparison/evidence/step8-review.json)
links all reports, validation logs/hashes, build manifests, restoration checks,
and pending platforms. Ignored binaries, native stacks, reproduction scripts,
source archives and logs remain under `target/performance-baselines/step8/` and
prior step directories. The paired driver source is embedded in its JSON, so the
measurement procedure survives removal of local build artifacts.

Native macOS 15 ARM64, Linux x86-64/ARM64, and Windows x86-64 validation remain
pending. This host is macOS 27.0.1; deployment targeting 15.0 does not establish
native baseline or release support. Dist-profile/package/dependency/archive/
extracted-package validation was not performed, and compatibility claims were
not expanded. Local test/lint success is not release evidence.

Next-step summary: the authorized optimization sequence is complete. Optional
future work should profile the remaining AST lookup/call and owned-collection
costs and propose focused improvements before coding; the rejected engine is not
an active feature. Keep this temporary checklist until the user explicitly
requests or approves its removal. **Final result acceptance: pending user review.**

## Completion record template

Add a record beneath the completed step, then update the current checkpoint:

```text
Status: Ready for review
Changed:
Checks run and results:
Measurements and evidence paths:
Regressions, risks, or pending validation:
Recommended next step and its scope:
User acceptance / authorization to continue: Pending
```

Record actual user acceptance separately from test success. Do not mark work
accepted or authorize the next step on the user's behalf.
