# Runtime optimization map

This is a source map of the current runtime paths, their selection boundaries,
and the evidence used to check them. Optimization bounds below are selection or
retention bounds; they do not introduce language limits. See
[architecture](architecture.md#execution-hot-paths-and-limits) for the runtime
overview and [language semantics](language-spec.md) for observable behavior.

## Selection entry points

Selection stays at the existing evaluator boundaries in
[`lib.rs`](../crates/scr-runtime/src/lib.rs):

1. **User calls:** `call_function_inner` establishes ownership, arity, source,
   frame and parameter scope. Non-method calls without a value-byte ceiling try
   `numeric_call`, then `prepared_call`, then ordinary parameter binding/body
   evaluation. Prepared recursion additionally declines configured scope limits.
2. **For loops:** after entering the loop scope and evaluating initializers,
   `statement` tries `integer_loop`, `float_loop`, then `numeric_call_loop`.
   A declined plan leaves the ordinary loop to continue from that setup.
3. **Assignments:** discarded, ungrouped variable-to-variable assignments select
   `copy_validation::assignment`. That handler evaluates the place/RHS once and
   can pass the resulting value to `write_place`. Other assignments use
   `assignment`, which can reuse an eligible string buffer.
4. **Properties/indexes:** direct property calls can use class-owned method
   hints; field and map access can use checked positions. Misses use ordinary
   resolution against the current object or collection.

For call/loop plan helpers, `None` declines selection and `Some(result)` handles
the operation, including any failure. This is distinct from a lookup hint miss
or the copy handler's fallback with an already evaluated RHS.

## Execution shortcuts

| Path and source entry | Eligibility and owned state | Decline/fallback boundary |
| --- | --- | --- |
| Numeric leaf calls — [`numeric_call`](../crates/scr-runtime/src/numeric_call.rs) | One pure numeric return using literals/parameters; finite numeric arguments; no defaults/variadics; at most 8 parameters, 64 expression nodes, depth 16. `UserDeclaration` owns the warm flag and cached plan or unsupported result. Plans retain no runtime values/scopes. | First production call stays ordinary; later calls may compile once. Unsupported syntax/arguments decline before annotations/body execution in the plan. General annotation resolution still handles mismatches and non-specialized annotations. |
| Integer self-recursion — [`prepared_call`](../crates/scr-runtime/src/prepared_call.rs) | After cached numeric-leaf rejection: exact current global callable, same runtime, no receiver, all entry arguments integers, absent/int annotations, no defaults/variadics. A simple parameter-versus-integer comparison selects two pure return paths; one must evaluate a self call. Declaration-owned plan/unsupported cache; at most 8 parameters, 128 nodes, depth 16. | Cached base cases decline without engine allocation. Callable identity is checked on entry; scope/value ceilings, non-integer inputs, captures, methods, mutual/dynamic/native calls and unsupported shapes stay ordinary. Selected execution uses explicit frames and a shared private scalar stack. |
| Integer loops — [`integer_loop`](../crates/scr-runtime/src/integer_loop.rs) | Supported comparisons/arithmetic over existing integer bindings; no calls or binding creation. Plans are local to this loop execution: at most 16 slots, 128 expression nodes, depth 32, and a 32-statement/action budget. Supports bounded blocks/conditionals as well as assignment-only bodies. | Configured value-byte ceilings and unsupported shapes decline before iterations. Completed local writes are published when the selected loop finishes or fails. |
| Float/numeric loops — [`float_loop`](../crates/scr-runtime/src/float_loop.rs) | Existing finite numeric bindings with stable kinds, at least one float, compatible comparisons/assignments. The privately identified core float converter is the only folded call. Per-loop plan: 16 slots, 128 nodes, depth 32, at most 32 body assignments. | Value-byte ceilings, untrusted converters, kind changes and unsupported calls/control flow decline. Ordinary evaluation is retained for those loops; selected loops publish completed writes on exit/failure. |
| Integer loops containing leaf calls — [`numeric_call_loop`](../crates/scr-runtime/src/numeric_call_loop.rs) | Integer counter `<` integer literal/binding, `+= 1`, estimated span at least 128 iterations. Up to 32 body assignments, 16 slots, 128 expression nodes, depth 32, 8 direct call nodes and 8 arguments per call. Callees must be same-runtime non-method functions proven integer-only leaves. | Value-byte ceilings, short/unsupported loops and unproven callees decline. The temporary plan retains resolved callables through loop completion, invokes normal call handling, and publishes completed caller writes on failure. No persistent target cache is installed. |
| List-copy validation — [`copy_validation::assignment` / `validate`](../crates/scr-runtime/src/copy_validation.rs) | Discarded plain variable copies, list length at least 32, final element int/bool/null. Counts the primitive suffix and validates the remaining prefix; private `CopyKey` keeps its generated hashing code separate. No persistent cache. | Other values/shapes use `write_place` with the evaluated value. Used results, grouped RHS, indexed/property targets and other contexts retain general assignment. Validation failure propagates before committing the destination. |
| String-buffer reuse — `assignment` / `self_concat_suffix` in [`lib.rs`](../crates/scr-runtime/src/lib.rs) | Discarded direct-variable `.=` with a pure suffix, or recognized `$s = $s . suffix`, when the destination is already a string. Owns the evaluated suffix before mutating the existing buffer; no cache. | Other contexts use general assignment. Once selected, suffix type, size/allocation or budget failures propagate without reevaluating the suffix or committing an append. |
| Large map literals — [`map_literal` / `indexed_map_literal`](../crates/scr-runtime/src/map_builder.rs) | 96–4096 entries with literal int/string keys and at most 256 KiB of literal key text. The first 8 unique entries use prefix search; a transient index borrows AST keys and stores positions, never values. The ordered `Vec` remains canonical. | Other literals use linear construction. Index reservation failure drops metadata and continues construction with linear lookup, preserving evaluated entries, duplicate replacement and order. The index dies with this construction. |

## Checked hints, storage and collection

| Path and source entry | State, validation and lifetime | Ordinary path |
| --- | --- | --- |
| Map positions — [`MapIndexes::position` / `invalidate`](../crates/scr-runtime/src/map_index.rs), called by `variable_ref` / `write_place` | Runtime-owned LRU hints for the first indexed step of variable-root maps with 64–4096 entries. At most 8 indexes, 4096 total indexed entries and 256 KiB of names/key text. Each hit checks the current stored key; whole-variable writes invalidate by scope/name. | Missing/stale positions never establish absence. Reads use linear lookup and can rebuild; writes consult existing hints without rebuilding on every insert. Reused scope IDs/replaced maps still require current-key validation. |
| Direct method calls — [`resolve_property` / `call_property`](../crates/scr-runtime/src/objects.rs), [`MethodCache`](../crates/scr-runtime/src/method_cache.rs) | Class-owned 8-slot span hints hold owner depth/method index, no receivers or declaration owners. Hits recheck owner bounds and method name. Hits repay misses; 32 accumulated misses disable the class cache. Direct calls avoid constructing a bound callable and protect receiver/lexical owners during arguments. | Default/empty spans, collisions, invalid hints, fields and native properties retain ordinary resolution. Current class/runtime identity and property behavior remain checked. |
| Field positions and writes — [`fields` / `object_field` / `set_object_field`](../crates/scr-runtime/src/objects.rs) | Immutable declaration order supplies positions; ordinary construction stores initialized fields in `Bindings::Slots`, including wide objects. Binding access checks both position and name. Writes normally measure live fields; nested bytes/object handles make the object's snapshot-measurement flag sticky. | Absent/uninitialized fields and promoted stores use name lookup. Exception storage keeps its special handling; native properties use the host interface. These are checked positions, not unchecked cached addresses. |
| Small bindings — [`Bindings::resolve` / `get_slot` / `insert_parameter`](../crates/scr-runtime/src/bindings.rs) | Each scope has one canonical store. Small variable/parameter stores use up to 8 bindings; parameter names can share declaration-owned `Rc<String>` values. Hints recheck name/bounds after callbacks or store changes. | Inserting beyond the small-store bound promotes to hashing; adding an ordinary binding to a parameter store can promote earlier. Slot misses use name lookup; stores do not automatically demote. Object construction can deliberately install wider ordered slot storage. |
| Scope storage — [`enter` / `enter_empty` / `leave_parameter` / `leave_block`](../crates/scr-runtime/src/environment.rs) | Runtime-owned pools: 8 empty block records, plus a shared pool of at most 64 spare/reserved records bounded to 32 retained collection slots per entry. Reuse clears values, parents and flags; reserved records are not live scopes. Warm numeric parameter reuse is admitted by `can_reuse_numeric_parameter_scope`. | Captured scopes remain traced. Namespace storage, oversized/nonempty ineligible records, pool ceilings or failed reservation use ordinary allocation/release. Reuse is storage policy, not permission to skip logical scope limits. |
| Call metadata and argument buffers — [`UserDeclaration`](../crates/scr-runtime/src/value.rs), `take_argument_buffer` / `call_function` in [`lib.rs`](../crates/scr-runtime/src/lib.rs), [`ActiveFrame`](../crates/scr-runtime/src/error.rs) | Calls share immutable declaration/name metadata. Runtime-local argument reuse retains at most 64 cleared buffers, each with capacity at most 16. Suspended calls own their current arguments; traces copy names into public diagnostics when needed. | Oversized/excess buffers are dropped. Defaults, variadics, receivers, source switches and errors retain ordinary call setup and cleanup. |
| Primitive annotations — `annotation` in [`lib.rs`](../crates/scr-runtime/src/lib.rs), numeric annotation checks in [`numeric_call.rs`](../crates/scr-runtime/src/numeric_call.rs) | Successful built-in value/type matches can skip class lookup; numeric plans preclassify absent/int/float annotations in declaration-owned metadata. | General/qualified types, mismatches and native objects named like primitives keep normal resolution and diagnostics. |
| Collection scheduling — `checkpoint` in [`lib.rs`](../crates/scr-runtime/src/lib.rs), [`capture_type` / `collect_if_needed` / `collect`](../crates/scr-runtime/src/environment.rs) | The runtime owns allocation debt, a threshold and weak function/object registries. New tracked functions/objects and newly captured local type environments add debt. After tracing, debt resets and the threshold becomes at least 256, scaling with live scopes and tracked live functions. | Checkpoints skip scans below the threshold. Execution boundaries, explicit `collect_garbage` and collection before rejecting scope allocation still trace directly; pacing is not a replacement for those collections. |

## Shared behavioral contracts

Ordinary evaluation is the semantic reference. These contracts apply to every
selected path; the tables above identify the guards that make each shortcut safe.

| Contract | What an optimized path must preserve |
| --- | --- |
| Evaluation and types | Callee, arguments, operands, keys/values and assignment places keep their specified order and execute once. Defaults, annotations, short-circuiting, strict numeric kinds, checked integer operations and finite float results retain ordinary behavior. See [evaluation](language-spec.md#runtime-values-scopes-and-execution) and [operators](language-spec.md#strict-operators-and-conversions). |
| Errors and control flow | Preserve error kind, precedence, redaction, failing span, definition source and caller frames. A selected execution error propagates; it cannot trigger another execution strategy. Return, throw, exit and interruption keep their distinct signals and unwinding behavior. See [exceptions](language-spec.md#exceptions-and-control-flow), [diagnostics](language-spec.md#output-exit-and-errors) and [native errors](standard-library.md#runtime-context-and-errors). |
| Mutation and visibility | Validate a replacement before committing the failing assignment. Earlier completed writes and RHS effects remain observable. Loop plans publish completed dirty slots on success or failure before surrounding code can observe them; their guards exclude callbacks that could observe stale local snapshots. Copy fallback keeps its evaluated value. Map construction preserves entry order and replaces a duplicate at its original position. |
| Budgets and cancellation | Keep logical statement/back-edge checkpoints and their spans, including empty loops; these count evaluator steps, not private opcodes. `checkpoint` checks cancellation before the instruction ceiling, then collection and step accounting. Preserve call depth, live scopes, value bytes and nesting checks; unsupported configured ceilings select ordinary evaluation where listed above. Pool capacity and private stack size never replace logical accounting. See [execution limits](language-spec.md#output-exit-and-errors). |
| Ownership and cleanup | Keep declaration/caller sources alive through diagnostics and restore source, frames and scopes on every exit. Globals, captured environments, suspended callers, evaluator temporaries, direct-method receivers/owners and externally held values remain roots across collection. Metadata must not introduce hidden owning references. Opaque native cycles retain their documented host-cleanup requirement. Cancellation/fatal execution limits can bypass script cleanup; Rust ownership still releases unwound storage. See [lifecycle](language-spec.md#output-exit-and-errors) and [control flow](language-spec.md#exceptions-and-control-flow). |
| Native reentry | Reentrant calls share the runtime's output, ownership and execution budgets. Propagate the original `InvocationError` signal, including exits and interruptions, rather than converting it into a catchable error. Validate runtime/concrete handle identity before using internal IDs. See [callback registration and invocation](writing-extensions.md#calling-script-callbacks-from-a-native-function). |

### Selection and fallback boundaries

A declined call or loop plan has not executed its proposed body or iterations.
Common setup may already have happened: call arguments, arity/source/scope setup,
or loop initializers must not run again. Selection may warm declaration metadata
or discover unsupported syntax. `Some(result)` commits the helper to handling
that execution, including errors; it never means “try ordinary evaluation on
failure.” String reuse similarly propagates errors after selection without
replaying its suffix.

Copy assignment has a later boundary: it resolves the place and evaluates the
RHS before deciding whether specialized validation applies. Declining validation
passes that same value to `write_place`. Lookup hints instead fall back to
current name/key/property resolution when validation misses. Map-constructor
index reservation failure discards only the index and continues with existing
entries. None of these fallbacks restart script effects.

### Cache validity and intentional separation

Declaration plans distinguish cold/warming, compiled-supported and
compiled-unsupported states. Numeric leaf calls warm before compiling; prepared
recursion waits for cached leaf rejection. An unsupported syntax result belongs
to the immutable declaration. An input-dependent miss (argument kind, base-case
guard, configured limits or current callable identity) declines that invocation
without marking supported syntax permanently unsupported. Cached plans still
need current entry guards. Per-loop plans end with their execution; runtime and
class hints retain only the bounded state described above. Names, spans, scope
IDs and numeric positions locate candidates; they do not alone prove current
callable, receiver, binding or key identity.

Private representations also have boundaries: prepared recursion's `Scalar`
holds only int/bool/null, and numeric loops use their numeric storage without
changing public `Value` or collection ownership. Borrowed `CopyKey` separates
specialized validator hashing from the general `HashSet<&MapKey>` implementation
without copying key text. Earlier profiling/timing experiments retained these
separations to avoid disturbing clone/drop and hashing code generation in general
paths; even changing the numeric-leaf helper's signature caused a control-workload
regression and was reverted. The existing `numeric_call(declaration, values)`
boundary therefore remains deliberate. Arithmetic helpers are shared where their
semantics match; broader representation/helper unification needs fresh matched
measurements. Historical timings remain local evidence, as described below.

## Tests and benchmark coverage

Links below identify existing checks and useful timing workloads, not new test or
performance results. Workloads combine paths; a timing row alone does not prove
that a particular guard was entered. Named standard workloads live in the
[CLI comparison fixtures](../benchmarks/comparison/fixtures); they can also run
through the [execution-only worker](../crates/scr-cli/examples/comparison_profile.rs).

| Area | Existing correctness checks | Useful timing coverage |
| --- | --- | --- |
| Numeric calls and annotations | [`numeric_call` tests](../crates/scr-runtime/src/numeric_call), [`call_setup_tests.rs`](../crates/scr-runtime/src/call_setup_tests.rs) | `function_calls`; extended `numeric_calls`. |
| Prepared recursion | [`prepared_call/tests.rs`](../crates/scr-runtime/src/prepared_call/tests.rs) | `recursive_fib`; additional self/mutual/method/deep/base-case controls currently live in ignored experiment evidence. |
| Loop plans | Unit tests in [`integer_loop.rs`](../crates/scr-runtime/src/integer_loop.rs), [`float_loop.rs`](../crates/scr-runtime/src/float_loop.rs), [`numeric_call_loop/tests.rs`](../crates/scr-runtime/src/numeric_call_loop/tests.rs) | `numeric_loop`; extended `numeric_float`, `numeric_nested`, `numeric_bounds`, `numeric_calls`. |
| Copy validation and strings | [`copy_validation_tests.rs`](../crates/scr-runtime/src/copy_validation_tests.rs), [`string_assignment.rs`](../crates/scr-runtime/tests/string_assignment.rs), [`collection_hot_paths.rs`](../crates/scr-runtime/tests/collection_hot_paths.rs) | `string_concat` exercises string reuse. `list_pipeline` and the [retained-copy fixture](../benchmarks/fixtures/memory.scripta) are general collection/copy controls, not dedicated variable-copy coverage. Plain-variable/prefix copy timings remain in ignored evidence. |
| Map construction and hints | [`map_builder_tests.rs`](../crates/scr-runtime/src/map_builder_tests.rs), unit tests in [`map_index.rs`](../crates/scr-runtime/src/map_index.rs), [`cached_program.rs`](../crates/scr-runtime/tests/cached_program.rs) | `dict_ops` covers lookup/growth/copy-removal. Dedicated large-literal timings remain in ignored evidence; the standard dictionary workload does not exercise the indexed literal constructor. |
| Methods and fields | [`method_cache_tests.rs`](../crates/scr-runtime/src/method_cache_tests.rs), [`field_update_tests.rs`](../crates/scr-runtime/src/field_update_tests.rs), [`method_dispatch.rs`](../crates/scr-runtime/tests/method_dispatch.rs), root-protection test in [`objects.rs`](../crates/scr-runtime/src/objects.rs) | `class_dispatch`; separate field/wide-object/inheritance timings remain in ignored evidence. |
| Bindings, calls and scope reuse | Unit tests in [`bindings.rs`](../crates/scr-runtime/src/bindings.rs), [`environment.rs`](../crates/scr-runtime/src/environment.rs), [`parameter_scope_tests.rs`](../crates/scr-runtime/src/environment/parameter_scope_tests.rs), [`call_reuse_tests.rs`](../crates/scr-runtime/src/call_reuse_tests.rs) | [Interpreter harness](../crates/scr-cli/benches/interpreter.rs): `variables`, `calls`, `loops`; comparison call/recursion controls. |
| Ownership and host boundaries | [`memory_lifecycle.rs`](../crates/scr-runtime/tests/memory_lifecycle.rs), [`invocation.rs`](../crates/scr-runtime/tests/invocation.rs), [`resources.rs`](../crates/scr-runtime/tests/resources.rs), [`block_scope_tests.rs`](../crates/scr-runtime/src/block_scope_tests.rs) | [Collector worker](../crates/scr-runtime/examples/collector_bench.rs) / [driver](../benchmarks/collector.py), [memory-soak driver](../benchmarks/memory_soak.py). These complement timing tests with lifecycle checks. |

Reproduction and interpretation: [CLI/API comparison](../benchmarks/comparison/README.md),
[extended workloads](../benchmarks/extended/workloads.py), and
[benchmark/memory guidance](../benchmarks/README.md). Historical experiment files
under ignored `target/performance-baselines/` are local evidence and may be removed
by build cleanup; they are not committed regression fixtures.
