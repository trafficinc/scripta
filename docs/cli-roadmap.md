# Scripta CLI development phases and approvals

Scripta's next work focuses on practical CLI automation: processing text,
transforming collections, handling failures, and reusing scripts. Each phase
requires explicit approval before implementation starts. Completing one phase
does not authorize the next.

**Next action: review Phase 3 Recoverable errors.** Implementation is ready for
review. Acceptance and approval to start Phase 4 are pending.

## Progress and approval record

| Phase | Scope | Status | Approval to start | Completion accepted |
| --- | --- | --- | --- | --- |
| 1 | Strings | Accepted | Approved | Accepted |
| 2 | Collections and maps | Accepted | Approved | Accepted |
| 3 | Recoverable errors | Ready for review | Approved | Pending |
| 4 | Functions | Waiting for Phase 3 | Pending | Pending |
| 5 | Math and formatting | Waiting for Phase 4 | Pending | Pending |
| 6 | Cleanup and streaming | Waiting for Phase 5 | Pending | Pending |

Record explicit approvals here as they arrive. Use the statuses Waiting,
Approved, In progress, Ready for review, and Accepted. Record the approving
instruction and any scope changes with the relevant phase. Approval to start
and acceptance of completed work are separate decisions; one message may give
both acceptance of a finished phase and approval to start the next.

## Work already implemented

List growth supports `$items[] = value`, assignment at exactly the list's current
length, and appending to existing nested lists. Sparse writes remain errors.
An [arrays playground](../playground/arrays.scr) demonstrates the behavior.
Workspace tests, Clippy, and formatting checks passed for this implementation.
This work is the baseline for the phases below.

## Rules for every phase

1. Obtain explicit approval to start the named phase.
2. Define the public API and behavior before coding, including argument types,
   return values, error cases, Unicode behavior where relevant, and copy or
   mutation semantics. Ask for approval if the agreed scope materially changes.
3. Implement only the approved phase, preserving strict conversions, collection
   value semantics, and the CLI focus.
4. Add focused tests for behavior and relevant resource limits, plus a runnable
   playground example. Update the language specification, standard library
   reference, export counts where affected, and README basics review.
5. Run the applicable workspace tests, formatting checks, and Clippy. Record the
   commands, results, and material limitations in the phase's completion record.
6. Mark the phase Ready for review, summarize its behavior and demonstration
   command, then stop implementation until the user accepts it and approves the
   next phase. Do not infer approval from silence or passing tests.

Committing, pushing, publishing releases, and changing GitHub settings are not
part of these phase approvals unless separately requested.

## Phase 1 Strings

Make everyday text processing available without launching an external program.

**Scope**

- Add `String::trim`, `split`, `join`, `contains`, `startsWith`, `endsWith`,
  `replace`, `substring`, `lower`, and `upper`.
- Use literal strings for splitting, searching, and replacement; regex support
  is deferred.
- Use Unicode scalar positions for substring operations, consistent with `len`.
  Document Unicode whitespace and case conversion behavior.
- Specify empty-input behavior, reject empty split delimiters, and require
  string elements for joining. Return new values and enforce applicable limits.

**Completion checks**

- Every function has a documented signature and examples.
- Tests cover ASCII and Unicode, empty strings, missing matches, invalid
  arguments, substring bounds, and output limits.
- A text-processing playground parses and normalizes sample input.
- The shared phase checks pass.

**API contract**

- Unary helpers take a string; predicates take `(text, search)`.
- `split(text, separator)` preserves empty fields; `join(items, separator)`
  accepts only strings in its list. Empty separators are valid for joining.
- `replace(text, search, replacement)` replaces all literal nonoverlapping
  matches. Empty split separators and replacement searches raise `RangeError`.
- `substring(text, start, length)` uses nonnegative Unicode scalar offsets;
  length is optional and defaults to the remaining text. Bounds are strict;
  start at the end is valid for an empty result. Invalid bounds raise `RangeError`.
- Trimming uses Unicode whitespace. Case conversion uses locale-independent
  Unicode mappings. Comparisons are case-sensitive and do not normalize Unicode.
- Helpers return new values. Output budgets include Value slots and UTF-8 bytes;
  split includes both its list slot and every field slot. Detected size
  overflows, budget violations, and fallible reservation failures raise
  `ResourceLimitError`.

**Approval record:** Approved by the user's “ok continue” instruction after the
phase plan was delivered. This approval applies only to Phase 1.

**Acceptance record:** Accepted by the user's “ok go to phase 2” instruction.

**Completion record:** Accepted. Phase 1 added ten String helpers, bringing the
standard library to 83 functions at that milestone. The
[reference](standard-library.md#strings), [language specification](language-spec.md),
and README describe their signatures and behavior. Focused tests cover Unicode
case expansion, combining marks, literal matching, empty fields, invalid types
and bounds, exact logical output budgets, cancellation, and failed assignments.
The CLI test runs the playground from an unrelated directory without external
programs and verifies its output.

| Validation | Result |
| --- | --- |
| `cargo test --workspace --locked` | Passed, including String unit tests, eight String integration tests, and the playground CLI test. |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Passed. |
| `cargo fmt --all -- --check` | Passed. |
| `git diff --check` | Passed. |
| `cargo run -p scr-cli --locked -- run playground/strings.scr` | Passed; normalized a Unicode status record and printed successful search checks. |

Review the [strings playground](../playground/strings.scr) with the command above.
Matching is literal and case-sensitive, substring positions count Unicode scalars
rather than grapheme clusters, and configured byte limits are logical per-value
limits. Regex, normalization, and string indexing are outside this phase.

**Next approval after acceptance:** Phase 2 Collections and maps.

## Phase 2 Collections and maps

Support common data transformations and safe map access.

**Scope**

- Add list removal, slicing, concatenation, membership, and sorting helpers.
- Add map construction, key existence checks, lookup with a default, keys,
  values, and removal helpers.
- Prefer helpers that return new collections, preserving existing value
  semantics. Define whether a returned list preserves or renumbers indexes.
- Define sorting for supported scalar types without implicit coercion; specify
  behavior for mixed types, empty collections, and invalid positions.
- Keep integer and string map keys distinct. Document missing-key and removal
  behavior explicitly.
- Defer callback-based map/filter/reduce until function support is reviewed.

**Completion checks**

- A script can construct an empty map and safely read an optional key with a
  default, without relying on JSON conversion or a manual search.
- Tests cover copy independence, ordering, equality, bounds, key types, and
  applicable nesting and byte limits.
- A collection playground filters, sorts, and summarizes sample records.
- The shared phase checks pass.

**API contract**

- `List::remove(items, index)` returns a copy without an existing position;
  indexes are renumbered contiguously from zero. Invalid positions raise `RangeError`.
- `List::slice(items, start, length)` uses strict nonnegative bounds and an
  optional length defaulting to the remaining elements, like String substring.
- `List::concat(left, right)` returns both lists' elements in order.
- `List::contains(items, value)` uses Scripta's strict structural equality,
  including typed numeric values and native object/callable identity.
- `List::sort(items, descending)` is stable and ascending by default. The
  optional direction is bool. Empty lists are valid; nonempty inputs must
  contain only ints, only finite floats, or only strings. Mixed and unsupported
  types raise `TypeError`; string order matches the language's comparisons.
- `Map::create()` returns an empty map. `has(map, key)` tests key existence;
  `get(map, key, default)` returns a value or the optional default (null when
  omitted). An existing null remains null. Arguments, including defaults, are
  evaluated eagerly as for other native calls.
- `Map::keys(map)` and `values(map)` return lists in insertion order.
  `Map::remove(map, key)` returns a copy without that key; an absent key is a no-op.
- Map keys must be int or string, with no coercion. Helpers return independent
  collection values and check applicable nesting and logical output byte limits
  before copying results. Detected size/budget/reservation failures raise
  `ResourceLimitError`. Callback-based transforms and pop are deferred.

**Approval record:** Approved by the user's “ok go to phase 2” instruction.
This also accepts Phase 1. Approval applies only to Phase 2 and its log analyzer
checkpoint.

**Acceptance record:** Accepted by the user's “continue” instruction after the
Phase 2 completion summary.

**Completion record:** Accepted. All five List helpers and six Map
helpers are registered by default, bringing the standard library to 94 functions.
The [reference](standard-library.md#collections), language specification, and
README describe the APIs. Nine collection integration tests and two focused
unit tests cover order, typed keys, stored nulls, eager defaults, deep copies,
native identity, stable signed-zero sorting, invalid types/bounds, exact output
budgets, nesting limits, cancellation, and failed assignment behavior. The CLI
test covers both new playgrounds, CRLF input, malformed records, an empty file,
and the log analyzer's usage exit code.

| Validation | Result |
| --- | --- |
| `cargo test --workspace --locked` | Passed, including collection unit/integration tests and both playgrounds. |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Passed. |
| `cargo fmt --all -- --check` | Passed. |
| `git diff --check` | Passed. |
| `./target/debug/scr run playground/collections.scr` | Passed; filtered, sorted, sliced, combined, and summarized sample records. |
| `./target/debug/scr run playground/log-analyzer.scr playground/sample-events.txt` | Passed; reported three info, one warn, one error, three unique services, and one malformed record. |

Review the [collections playground](../playground/collections.scr) and
[log analyzer](../playground/log-analyzer.scr). Rebuild the interpreter with
`cargo build -p scr-cli --locked` before using the commands above on another
checkout. Collection helpers return new values; sorting accepts homogeneous
numeric or string lists. Byte budgets are logical per-value limits and do not
bound sorting scratch space.

**Practical checkpoint:** Build a small log analyzer using the string and
collection APIs. Review friction it exposes before approving Phase 3; any new
feature work needs an explicit scope decision.

The checkpoint is implemented with a small synthetic events file. It accepts
exactly three pipe-separated fields, `level|service|message`, with levels info,
warn, or error matched without case sensitivity. It skips comments and blank
lines, counts malformed records, and lists unique sorted service names. Fields
do not support quoting or embedded pipes. `Map::get` makes counting missing keys
straightforward, while explicit loops handle filtering and deduplication.
Whole-file reading remains appropriate for this small example; streaming and
callback transforms remain deferred. No additional features were needed for
the checkpoint.

**Next approval after acceptance:** Phase 3 Recoverable errors.

## Phase 3 Recoverable errors

Let scripts recover from ordinary data and arithmetic failures.

**Scope**

- Introduce documented catchable errors for missing map keys, invalid list
  indexes, and arithmetic failures such as division by zero and overflow.
- Review type failures in the same operations for consistent exception names.
- Preserve source locations, stack traces, and diagnostic redaction.
- Preserve uncaught-error exit behavior. Cancellation and execution-budget
  failures must continue to bypass script catches.
- Document the change from today's behavior, where these core failures bypass
  `catch`.

**Completion checks**

- Scripts can catch the named errors and continue with a fallback.
- Nested calls retain useful diagnostics; failed writes leave collections in
  the documented state.
- Tests confirm that catch blocks cannot swallow cancellation or budget limits.
- An error-handling playground demonstrates successful recovery and an
  uncaught-error example documents the resulting exit status.
- The shared phase checks pass.

**API contract**

- Core missing-key reads, out-of-bounds list reads/writes, and negative list
  positions raise `IndexError`. Sparse list writes still fail.
- Division/remainder by zero, checked integer overflow, unary negation overflow,
  and non-finite float arithmetic results raise `ArithmeticError`.
- Operator/condition type mismatches, invalid index/key types, indexing a scalar,
  and appending to a non-list raise `TypeError`. Strict conversions are unchanged.
- Named catches, `catch (Error $e)`, and untyped catches match these failures;
  bindings contain redacted message strings, not exception objects. A string
  typed catch does not match an exception merely because its message is a string.
- Uncaught errors keep diagnostic code E0401, original spans and stack frames,
  and CLI exit status 1, with the exception name prefixed to the message.
- Core runtime cancellation, instruction/scope/value/source/depth limits, and
  allocation failures remain outside catches. Existing native exceptions such
  as helper `ResourceLimitError` retain their current behavior.
- Failed operations do not commit a destination write. Earlier target/RHS side
  effects remain visible, matching existing evaluation order and assignment rules.
- Undefined names, invalid call targets, and other unrelated runtime failures
  remain outside this phase.

**Approval record:** Approved by the user's “continue” instruction after the
Phase 2 completion summary. This also accepts Phase 2 and its checkpoint.

**Completion record:** Ready for review. Core operators and indexing now return
typed error categories, separating recoverable TypeError/IndexError/ArithmeticError
from fatal resource failures without inspecting message text. Core exceptions
reuse the original diagnostic span, source, redacted message, and stack frames.
Existing native exception messages and explicit throws retain their prior
semantics. No new standard-library functions were needed; the export count
remains 94.

Seven new runtime integration tests cover named and generic catches, negative
and missing positions, both optimized and general reads, arithmetic boundaries,
operator/condition types, compound writes, RHS side effects, unmatched catches,
explicit string rethrows, diagnostic redaction, runtime reuse, and core safety
limits/cancellation. Additional coverage checks native-property indexing,
resource release during core-error unwinding, fatal allocation classification,
and CLI recovery versus uncaught-error diagnostics and status.

| Validation | Result |
| --- | --- |
| `cargo test --workspace --locked` | Passed, including the recoverable-error and playground tests. |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Passed. |
| `cargo fmt --all -- --check` | Passed. |
| `git diff --check` | Passed. |
| `./target/debug/scr run playground/errors.scr` | Passed; recovered from missing-key, arithmetic, and operand-type errors, preserving the failed compound assignment's destination. |
| `./target/debug/scr run playground/errors.scr --uncaught` | Expected exit status 1 with E0401, IndexError, source location, and a main stack frame. |

Review [the errors playground](../playground/errors.scr) and the
[exception contract](language-spec.md#statement-grammar). Previously fatal
operator/index failures are now catchable, and their uncaught diagnostic
messages gain an exception-name prefix. Core execution limits, cancellation,
undefined names, invalid call targets, and unrelated fatal errors remain outside
catches. Failed writes do not roll back earlier target/RHS side effects.

**Next approval after acceptance:** Phase 4 Functions.

## Phase 4 Functions

Make reusable helpers easier to call and pass around.

**Scope**

- Add default parameter values, then variadic parameters and anonymous function
  literals within this phase.
- Define default evaluation timing and scope, mutable-value independence,
  required/default parameter ordering, and variadic argument types.
- Define anonymous-function capture and lifetime behavior consistently with
  existing named lexical captures.
- Preserve positional calls and type annotations. Named arguments and arrow
  function shorthand remain deferred.
- Review callback-based collection helpers after anonymous functions work;
  adding those helpers requires a separate scope approval.

**Completion checks**

- Defaults do not accidentally share mutable values between calls.
- Tests cover arity, annotations, variadic collection behavior, captures,
  recursion, and error traces.
- A functions playground uses an optional argument, variable-length input, and
  an anonymous function passed as a callback.
- The shared phase checks pass.

**Approval record:** Pending.

**Completion record:** Pending.

**Next approval after acceptance:** Phase 5 Math and formatting.

## Phase 5 Math and formatting

Support common calculations and readable CLI reports.

**Scope**

- Add `abs`, `min`, `max`, `floor`, `ceil`, and `round` through a documented
  numeric API consistent with the standard library's module conventions.
- Define integer versus float handling, return types, overflow behavior,
  negative values, and rounding ties. Preserve finite numeric values and
  explicit conversions.
- Add a bounded string-formatting helper with a small documented placeholder
  syntax, argument validation, and literal escaping rules.
- Keep interpolation, powers, and random-number APIs outside this phase.

**Completion checks**

- Boundary tests cover integer limits, finite floats, rounding ties, and invalid
  input types.
- Formatting tests cover escaped placeholders, missing or extra arguments,
  Unicode, and output limits.
- A reporting playground formats a readable numeric summary.
- The shared phase checks pass.

**Approval record:** Pending.

**Completion record:** Pending.

**Next approval after acceptance:** Phase 6 Cleanup and streaming.

## Phase 6 Cleanup and streaming

Provide reliable cleanup and bounded processing of large files.

**Scope**

- Add `finally` to the existing exception model before adding file streams.
- Define cleanup behavior for normal completion, throw, return, break,
  continue, and script exit. Specify how a failure inside cleanup interacts
  with an existing failure.
- Preserve prompt cancellation and execution limits; document cases where
  forced termination prevents script cleanup.
- Add explicitly managed file handles and bounded line reading or iteration.
- Preserve filesystem permissions and path policies, with documented encoding,
  line-ending, close, and EOF behavior.
- Keep whole-file APIs available for small files. Defer general resource
  management syntax and asynchronous I/O.

**Completion checks**

- Tests cover cleanup across supported control-flow paths and competing errors.
- Streams close explicitly and release native resources on runtime teardown.
- A large-file test demonstrates incremental reads with bounded buffers,
  including a configured maximum line size.
- Tests cover permission denial, read failures, EOF, encoding, cancellation,
  and runtime reuse after failure.
- A streaming playground processes a log file and uses `finally` for cleanup.
- The shared phase checks pass.

**Approval record:** Pending.

**Completion record:** Pending.

**Next decision after acceptance:** Review the CLI experience and choose further
work explicitly. There is no automatically authorized next phase.
