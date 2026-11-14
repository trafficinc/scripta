# Scripta CLI development phases and approvals

Scripta's next work focuses on practical CLI automation: processing text,
transforming collections, handling failures, and reusing scripts. Each phase
requires explicit approval before implementation starts. Completing one phase
does not authorize the next.

**Next action: review Phase 7 below.** All six
phases are accepted. Phase 7 is approved and implemented; completion is ready for review.

## Progress and approval record

| Phase | Scope | Status | Approval to start | Completion accepted |
| --- | --- | --- | --- | --- |
| 1 | Strings | Accepted | Approved | Accepted |
| 2 | Collections and maps | Accepted | Approved | Accepted |
| 3 | Recoverable errors | Accepted | Approved | Accepted |
| 4 | Functions | Accepted | Approved | Accepted |
| 5 | Math and formatting | Accepted | Approved | Accepted |
| 6 | Cleanup and streaming | Accepted | Approved | Accepted |
| 7 | CLI pipelines and help | Ready for review | Approved | Pending |

Record explicit approvals here as they arrive. Use the statuses Waiting,
Approved, In progress, Ready for review, and Accepted. Record the approving
instruction and any scope changes with the relevant phase. Approval to start
and acceptance of completed work are separate decisions; one message may give
both acceptance of a finished phase and approval to start the next.

### Package management (separate approved scope)

**Approval to start:** The user requested, "ok implement the package mamanger",
following the design for CLI-integrated package management in this repository.
This does not approve proposed Phase 7.

**Status:** Ready for review.

The CLI now provides `init`, `add`, `install`, and `update`, using `scripta.toml`,
versioned `scripta.lock` files, local snapshots, Git semantic-version tags or
revision pins, SHA-256 checksums, and private transitive dependencies. Script
loading resolves declared package names without downloading or changing the
existing shared-scope include behavior. See [packages](packages.md) for the API,
limits, failure behavior, and deferred registry/publishing features.

**Validation:** `cargo test --workspace --locked`,
`cargo clippy --workspace --all-targets --locked -- -D warnings`,
`cargo fmt --all -- --check`, and `git diff --check` pass locally on macOS.
Workspace tests need local port access for the existing loopback HTTP fixtures.
Seven package integration tests cover Git version selection/pins, transitive
isolation, local snapshots, offline installs, integrity checks, cycles, failed
installation preservation, path validation, and runtime permissions/policies.
The `examples/packages` demonstration installs with `--locked --offline`, prints
`Hello, packages!`, and both scripts pass `scripta fmt --check`. Linux/Windows
execution remains unverified by these local results.

### Aliased requires (separate approved scope)

**Approval to start:** The user requested “ok do this then” after choosing
`require_once "helpers" as Helpers;` with `Helpers::function()` calls, isolated
module scopes, and unchanged unaliased includes.

**Status:** Ready for review.

The optional `as Alias` works with both require forms and local files or installed
packages. Module functions/variables are isolated from the caller; aliases are
lexical and cannot shadow registered namespaces or bind a different file.
`require_once` shares a cached canonical-path instance, while `require` initializes
a fresh scope and publishes it on success. Dependencies retain source-relative
resolution and installed versions stay isolated. Alias and cache references are
traced by the collector, and cached roots count toward scope limits. Failed
initialization restores runtime state and preserves previous bindings; external
effects and already initialized dependencies are not rolled back. See
[package aliases](packages.md#package-aliases-and-isolation).

**Validation:** Local macOS ARM64 workspace tests passed with both
`--all-features` and `--no-default-features`, using `--locked --offline` and running
sequentially outside the sandbox for existing loopback HTTP/TLS fixtures. Eleven
focused module-alias tests cover duplicate names, scope isolation, shared once
state, fresh requires, conflict errors, nested includes/aliases, GC/callback
retention, cached empty modules, retry/fatal/exit cleanup, permissions/policy,
scope pressure, canonical identity, source diagnostics, and formatter/check
behavior. Installed-package tests verify two private dependency versions with
identical function and alias names, plus checksum enforcement. Parser tests cover
valid expressions/aliases and malformed bindings. The final self-entry once-state
fix also passed focused tests and the no-default-features workspace suite.

All-feature Clippy with warnings denied, Rust formatting, `git diff --check`, and
the package example's `scripta fmt --check` passed. The local package example installed
with `--locked --offline` and printed `Hello, packages!`. No standard-library
exports were added. These are local results, not Linux/Windows validation.

### Atomic file replacement (separate approved scope)

**Approval to start:** The user requested “ok do the plan” for
`File::writeAtomic(path, contents)`, sibling staging, explicit commit semantics,
permissions/policy checks, cleanup, failure/concurrency tests, and updated report
guidance. This does not authorize a separate source-file replace API.

**Status:** Ready for review.

`File::writeAtomic` stages bounded UTF-8 writes in the destination directory,
synchronizes/closes the staged file, and publishes through tempfile's native
same-filesystem replacement without a delete/copy fallback. Existing regular
files need write access; final symlinks, nonregular and read-only targets are
rejected. Unix ordinary mode bits are preserved (special bits stripped), new
Unix files start private, and Windows uses directory-inherited ACLs. Other
metadata is not preserved. Precommit failures/cancellation remove staging files
and preserve the destination; successful publication has no postcommit
cancellation check. There is no directory synchronization, hard durability
promise, multi-file transaction, or adversarial path-race sandbox.

The library now has 106 exports. The report example previews unless `--apply`
is given, and the static-web guide uses atomic writes with first-run reader
permissions. See the [contract](standard-library.md#atomic-file-replacement).

**Validation:** On macOS ARM64, `cargo test --workspace --all-features --locked
--offline` passed outside the sandbox for the existing loopback HTTP/TLS fixtures.
Five focused unit tests cover injected partial writes, commit failures,
precommit/postcommit cancellation, destination revalidation, and actual cleanup
failure propagation. Six filesystem integration tests cover Unicode/empty
creation/replacement, strict signatures, invalid targets, write/root permissions,
read-only files, Unix modes/symlinks/open descriptors, and concurrent-reader
old/new snapshots. The improved cleanup-failure test also passed its targeted
unit run after the workspace run. A Windows-only test holds a handle without
delete sharing and checks replacement failure preserves the original; this test
has not been executed on Windows here.

The CLI report example passed preview/no-write, create/replace, parsed JSON,
formatter equivalence, and no-staging-file checks. The export reference test now
expects 106 functions. All-feature Clippy with warnings denied, Rust formatting,
`git diff --check`, and `scripta fmt --check examples/atomic-report.scripta` passed.
These are local macOS results, not Linux/Windows validation.

### SQLite built-in extension (separate approved scope)

**Approval to start:** The user said "ok do it" after approving the plan for an
optional built-in SQLite crate, bound parameters, queries, transactions, resource
cleanup, permissions, limits, tests, and release instructions. This does not
approve proposed Phase 7.

**Status:** Ready for review.

`scr-ext-sqlite` now provides `Sqlite::open`, `execute`, `query`, and `close`.
The CLI's disabled-by-default `sqlite` feature includes bundled SQLite and
registers the functions per runtime. See [SQLite](sqlite.md) for types, error
categories, read-only mode, root policies, bounded/cancellable statements, and
unsupported SQL features. `examples/sqlite.scripta` demonstrates parameters and a
transaction. Verbose version output reports extensions; the release packager
accepts `--features sqlite`, includes matching notices, records extensions, and
checks the enabled SQL example during smoke validation.

**Validation:** On macOS ARM64, both
`cargo test --workspace --all-features --locked --offline` and
`cargo test --workspace --no-default-features --locked --offline` passed, run
sequentially so CLI binaries from different feature configurations do not race.
Existing HTTP fixtures require local port access. Eleven extension integration
tests cover conversions, parameter binding, constraints, transaction counts,
read-only connections, locking, roots and symlinks, denied SQL, limits,
cancellation, cleanup, and handle ownership. CLI tests cover both feature
configurations and repeat pipeline registration.

`cargo +1.85.0 check -p scr-ext-sqlite --locked --offline`,
`cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings`,
`cargo fmt --all -- --check`, `git diff --check`, and the SQLite example's
`scripta fmt --check` passed. Targeted release/extension tests passed after adding
extension-list archive validation and dangling-symlink coverage. A local archive
made from the debug SQLite-enabled binary passed verify and offline smoke;
its declared extensions and rusqlite/libsqlite3-sys license inventories were
checked. This is local development evidence, not a dist-profile release or
Linux/Windows validation result.

## Work already implemented

List growth supports `$items[] = value`, assignment at exactly the list's current
length, and appending to existing nested lists. Sparse writes remain errors.
An [arrays playground](../playground/arrays.scripta) demonstrates the behavior.
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
| `cargo run -p scr-cli --locked -- run playground/strings.scripta` | Passed; normalized a Unicode status record and printed successful search checks. |

Review the [strings playground](../playground/strings.scripta) with the command above.
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
| `./target/debug/scripta run playground/collections.scripta` | Passed; filtered, sorted, sliced, combined, and summarized sample records. |
| `./target/debug/scripta run playground/log-analyzer.scripta playground/sample-events.txt` | Passed; reported three info, one warn, one error, three unique services, and one malformed record. |

Review the [collections playground](../playground/collections.scripta) and
[log analyzer](../playground/log-analyzer.scripta). Rebuild the interpreter with
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

**Acceptance record:** Accepted by the user's “continue” instruction after the
Phase 3 completion summary.

**Completion record:** Accepted. Core operators and indexing now return
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
| `./target/debug/scripta run playground/errors.scripta` | Passed; recovered from missing-key, arithmetic, and operand-type errors, preserving the failed compound assignment's destination. |
| `./target/debug/scripta run playground/errors.scripta --uncaught` | Expected exit status 1 with E0401, IndexError, source location, and a main stack frame. |

Review [the errors playground](../playground/errors.scripta) and the
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

**API contract**

- Parameters use `$value: type = expression` for defaults and
  `...$values: type` for a variadic list. Required parameters precede defaults;
  a single variadic parameter must be last and cannot have a default.
- Supplied positional arguments evaluate left to right in the caller, as before.
  Missing defaults evaluate left to right once per call in the callee parameter
  scope, seeing earlier parameter bindings and captured enclosing function locals.
  Supplied arguments suppress their defaults. Explicit null does not omit an argument.
- Defaults can be expressions and are checked against their annotation after
  evaluation. Collection defaults are independent across calls; ordinary explicit
  side effects in default expressions are not rolled back on a later failure.
- Extra positional arguments become a fresh list, empty when no extras exist.
  A variadic annotation applies to each element. The resulting list is subject
  to the usual value nesting and byte limits.
- Anonymous literals use `function (...) { ... }` with optional return annotations.
  They are callable values, capture lexical environments by reference like named
  nested functions, and retain their definition source for diagnostics and includes.
  They do not register a function name; traces label them `<anonymous>`.
- Existing global isolation remains: pass globals explicitly; captures read and
  update enclosing function locals. Recursion through a captured local binding
  is supported. Captured scopes retain the existing lifetime and scope-budget rules.
- Arity and annotation errors remain catchable TypeError exceptions. Default
  evaluation failures preserve the callee source span and call stack. Core limits
  and cancellation retain their fatal behavior.
- Named arguments, arrow syntax, call-site unpacking, and callback-based standard
  library transforms remain deferred.

**Approval record:** Approved by the user's “continue” instruction after the
Phase 3 completion summary. This also accepts Phase 3.

**Acceptance record:** Accepted by the user’s “Phase 5 Math and formatting awaits
your approval. OK” instruction.

**Completion record:** Accepted. Named and anonymous functions support
per-call defaults, a final typed variadic parameter, and existing lexical
capture/return annotations. The lexer recognizes `...` for parameter declarations;
the parser and runtime reject invalid ordering. Anonymous literals register no
name and retain their definition source. `check` traverses defaults and function
literals inside expressions, and formatting preserves literal punctuation and
unbraced body indentation. The standard-library export count remains 94.

Eight runtime integration tests cover defaults, evaluation order, independent
collection values, variadic typing and wrapping, callbacks, shared captures,
recursion, global/caller isolation, arity/annotation failures, definition/call-site
traces, and cancellation/byte/depth/scope/instruction limits. Lexer/parser tests
cover ellipsis spans, parameter spans, malformed signatures, control-flow
boundaries, and expression budgets. Tooling tests cover nested anonymous
declaration validation and a formatter snapshot. A module test verifies relative
requires and traces inside callbacks retained across execute calls; the CLI test
checks the functions playground before and after formatting.

| Validation | Result |
| --- | --- |
| `cargo test --workspace --locked` | Passed, including new function, lexer/parser, tooling, module, and CLI tests. |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Passed. |
| `cargo fmt --all -- --check` | Passed. |
| `git diff --check` | Passed. |
| `cargo run -p scr-cli --locked -- run playground/functions.scripta` | Passed; printed greetings, total 16, callback result 14, counter values 6 and 8, and two independent defaults. |

Review [the functions playground](../playground/functions.scripta) and
[the language contract](language-spec.md#function-parameters-and-captures).
Defaults are evaluated only when omitted; explicit null remains an argument.
Variadic annotations check elements. Captures share enclosing function locals
and retain the existing scope lifetime and budget rules; globals still require
explicit arguments. Named arguments, arrow syntax, call-site unpacking, and
callback-based collection helpers remain deferred. Phase 5 is approved separately
below.

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

**API contract**

- `Math::abs(value)`, `floor(value)`, `ceil(value)`, and `round(value)` accept
  one int or finite float and preserve its type. Integers are unchanged except
  for abs; abs of i64 minimum raises `ArithmeticError`. Float floor rounds toward
  negative infinity, ceil toward positive infinity, and round to the nearest
  integer with halfway ties away from zero. Float results remain floats, including
  signed zero from rounding; abs converts negative zero to positive zero. No
  conversion to int is implied.
- `Math::min(left, right)` and `max(left, right)` require two ints or two finite
  floats and return the selected value with its original type. Equal inputs
  select the left argument, including signed zero. Mixed numeric types, nonnumeric
  values, non-finite floats, and wrong arity raise `TypeError`.
- `String::format(template: string, values: list): string` replaces each `{}`
  with the next scalar value using `str` display (null, bool, int, finite float,
  or string). `{{` and `}}` emit literal braces. Replacement strings are literal,
  not reparsed. Other brace syntax, unmatched braces, or missing/extra values
  raise `FormatError`; unsupported values raise `TypeError`.
- Formatting preserves Unicode and returns a new string. It preflights one Value
  slot plus output UTF-8 bytes before reserving its result, checks cancellation
  while scanning, and reports size overflow, budget or reservation failures as
  `ResourceLimitError`. It adds no precision, width, indexing, or interpolation
  syntax; numeric presentation uses the explicit Math and conversion helpers.

**Approval record:** Approved by the user’s “Phase 5 Math and formatting awaits
your approval. OK” instruction, which also accepts Phase 4. Approval applies only
to Phase 5.

**Acceptance record:** Accepted by the user’s “ok” after the Phase 5 completion summary.

**Completion record:** Accepted. Six Math helpers and `String::format`
are registered by default, bringing the standard-library export count to 101.
The reference, language specification, export coverage, and README describe the
new APIs. Numeric helpers preserve strict types and finite values; abs checks
integer overflow. Formatting validates brace syntax and scalar arguments, measures
output before reserving a result buffer, and checks cancellation during both
passes. No language syntax changes were needed.

Four integration tests cover numeric boundaries, positive and negative rounding,
signed zero, Unicode and escaped braces, strict types/arity, catchable failures,
failed assignment preservation, output limits, and runtime reuse. Unit tests
cover non-finite host values, exact output budgets (including empty output), and
cancellation. A CLI test checks the reporting playground, formatter stability,
and execution from an unrelated directory.

| Validation | Result |
| --- | --- |
| `cargo test --workspace --locked` | Passed outside the sandbox after an existing non-UTF-8 filesystem test was blocked with Operation not permitted inside it. |
| `cargo test -p scr-cli --locked --test release_examples reporting_playground_runs_before_and_after_formatting` | Passed after adding the reporting CLI test. |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Passed. |
| `cargo fmt --all -- --check` | Passed. |
| `git diff --check` | Passed. |
| `cargo run -p scr-cli --locked -- run playground/reporting.scripta` | Passed; reported four jobs, 8.6–19.5 ms duration, rounded average 13 ms, bounds 12–13 ms, and escaped literal braces. |

Review [the reporting playground](../playground/reporting.scripta),
[Math helpers](standard-library.md#math), and
[formatting contract](standard-library.md#strings). Formatting uses scalar display
without decimal precision, width, or indexed placeholders. Budgets are logical
per-value limits, not a process memory ceiling. No Phase 6 work has started.


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

**API contract**

- `try { ... }` requires one or more catch clauses, a final `finally { ... }`,
  or both. Finally runs once after the body and any matching catch, including
  normal completion, throw, return, break, continue, and exit. Its ordinary
  completion preserves the pending outcome; a throw, return, loop control, or
  exit from finally replaces that outcome. Diagnostics describe the replacement
  failure with its original source and stack. Finally has a separate block scope.
- Fatal runtime failures and cancellation bypass finally, as they bypass catches.
  Cleanup executes under the same instruction, depth, scope, value and cancellation
  limits; no extra execution allowance is granted. Native resources still release
  when their last reference is dropped, including runtime teardown unless the
  host retains aliases. OS termination may prevent all cleanup.
- `File::openLines(path: string, maxLineBytes: int optional): FileStream` opens a
  regular file for UTF-8 line reading; the default maximum is 1 MiB, measured in
  raw bytes per line including LF/CRLF terminators. The maximum must be positive.
  Opening uses the existing filesystem read capability, policy roots, and symlink
  rules. Stream operations retain the open descriptor and recheck read permission
  and host read policy, rather than reopening or resolving the path again. A
  stream belongs to its filesystem registration; reads through another
  registration raise `TypeError`. Its read-only bool `closed` property exposes
  whether the descriptor is closed.
- `File::readLine(stream: FileStream): string or null` returns the next line,
  stripping a terminal LF and its optional preceding CR. A bare CR is preserved.
  Blank lines return an empty string; EOF returns null, repeatedly. A final line
  without LF is returned normally; UTF-8 is strict and a BOM is preserved.
- `File::close(stream: FileStream): null` closes shared stream identity, is
  idempotent, and requires no read permission. Aliases see the same cursor and
  closed state. Reading a closed stream raises `StreamError`. Read/encoding/line
  size/allocation failures close the stream to avoid exposing a partial cursor.
  Cancellation within an active read also closes the stream.
- Reads use an 8 KiB buffer and grow only the current line, checking cancellation
  between chunks. Lines exceeding the configured raw-byte ceiling or the logical
  result limit (one Value slot plus UTF-8 bytes) raise `ResourceLimitError`.
  Invalid UTF-8 raises `EncodingError`; OS errors use existing filesystem error
  categories. Wrong types/arity raise `TypeError`, invalid maxima `RangeError`.
  These helpers add no writing streams, iterator syntax, or asynchronous I/O.

**Approval record:** Approved by the user’s “ok” after the Phase 5 completion
summary. This also accepts Phase 5. Approval applies only to Phase 6.

**Acceptance record:** Accepted by the user’s “continue” instruction after the
Phase 6 completion summary. This authorizes the planned CLI experience review;
it does not select an unspecified new implementation phase.

**Completion record:** Accepted. The lexer, AST, parser, runtime,
formatter, and declaration checker support finally after optional catch clauses.
Cleanup preserves pending outcomes on ordinary completion and replaces them when
it produces new control flow or failure; fatal runtime errors and cancellation
remain outside script cleanup. Catch binding validation occurs only for a selected
catch so an unmatched exception can reach finally without creating an unnecessary
binding budget failure. Diagnostics retain their source, stack, and redaction.

Three new File helpers provide shared, explicitly closable UTF-8 line streams,
bringing the default export count to 104. Reads retain an 8 KiB input buffer and
one bounded line, enforce raw-line and logical output limits, and close after
active read failures. The reference documents registration ownership, permissions,
line endings, EOF, errors, descriptor lifetime, and synchronous-read limitations.
The specification and README basics review describe the completed phase.

Eight runtime tests cover ordinary/caught/unmatched paths, competing failures,
returns and exits, nested loop control, scope separation, diagnostics/redaction,
fatal limits, cancellation, and runtime reuse. Lexer/parser/tooling tests cover
finally keywords and clauses, malformed syntax, declaration validation, formatter
stability, and preserved execution. Seven stream integration tests cover shared
cursors, explicit close, repeated EOF, Unicode/BOM/line endings, strict arguments,
line/output limits, failed assignment preservation, UTF-8 errors, permission and
root policy, symlinks, descriptor identity, and registration ownership. Three
stream unit tests cover exact output budgets across split UTF-8/CRLF buffers,
injected read failures/cancellation, and actual descriptor release on close and
runtime teardown. A 2 MiB input is processed under a 256-byte per-value budget,
while the same whole-file read fails. The CLI test runs the streaming playground
before/after formatting from another directory with sample, CRLF, empty,
invalid-encoding, and oversized-line inputs, and checks its usage exit status.

| Validation | Result |
| --- | --- |
| `cargo test --workspace --locked` | Passed outside the sandbox, including all cleanup, stream, tooling, playground, and export-index tests. |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Passed. |
| `cargo fmt --all -- --check` | Passed. |
| `git diff --check` | Passed. |
| `cargo run -p scr-cli --locked -- run playground/streaming.scripta playground/sample-events.txt` | Passed; reported five records (three info, one warn, one error), one malformed record, and a closed stream. |

Review [the streaming playground](../playground/streaming.scripta),
[cleanup semantics](language-spec.md#exceptions-and-control-flow), and
[stream contract](standard-library.md#bounded-file-streams). Cleanup receives no
extra execution budget, and fatal failures/cancellation can bypass it. Native
streams close when their last reference drops; a host-retained alias can extend
their lifetime past runtime teardown. OS reads remain synchronous and path races
retain the existing filesystem limitations. Writable streams, iterator syntax,
and general resource-management syntax remain deferred.


**Next decision after acceptance:** Review the CLI experience and choose further
work explicitly. There is no automatically authorized next phase.


## CLI experience review after Phase 6

**Review instruction:** The user requested “continue” after the Phase 6
completion summary. This review completes the next decision checkpoint; new feature work
still requires an explicit scope approval.

**Historical findings before Phase 7**

| Workflow | Observed result |
| --- | --- |
| `scripta help` | Exit 0 with global usage. |
| `scripta run --help`, `scripta check --help`, `scripta fmt --help` | Exit 2 with a filename/usage error instead of command-specific help. |
| `scripta help run` | Exit 2, unexpected extra arguments. |
| `scripta check playground/streaming.scripta` | Exit 0 without executing the script. |
| Reporting and streaming sample runs | Expected summaries; the streaming example reports five valid records, one malformed record, and a closed stream. |
| Piped data with `Cli::prompt("")` | Works, preserving Unicode and empty lines while stripping LF/CRLF; a 65537-byte payload raises `InputLimitError` at the prompt's fixed 64 KiB raw line limit. |
| Piped data into the streaming playground without a filename | Exit 2 with the example's usage message; it currently requires a file argument. |
| `scripta run -` | Exit 2; source from stdin is not implemented. This is separate from a file-backed script consuming piped data. |
| `SCR_MAX_STEPS=20 scripta run playground/streaming.scripta playground/sample-events.txt` | Exit 1 with the instruction-limit diagnostic and source/stack details. |
| `scripta fmt --check` on the new reporting/streaming playgrounds | Initially failed; both examples were formatted and now pass. Their execution tests still pass. |

The file-processing APIs cover the approved examples. The next practical
improvements are help discovery and a dedicated bounded input API for shell
pipelines. Existing `Cli::prompt("")` is a working stdin workaround, so pipeline
input is not wholly absent. A noninteractive API would avoid tying record input
to a prompt capability, fixed prompt line limit, and output flushing.

**Validation:** `cargo test -p scr-cli --locked --test release_examples playground`
passes all six matching playground tests. Both new playgrounds pass
`scripta fmt --check`. `cargo fmt --all -- --check` and `git diff --check` pass.
The CLI probes used temporary scripts/input and did not change public APIs.

## Phase 7 CLI pipelines and help

**Approval to start:** The user requested “fix:2. Proper shell-pipeline support”
and explicitly identified missing bounded stdin input, source from stdin, and
command-local help. This authorizes Phase 7 plus source-from-stdin execution.

**Status:** Ready for review.

**Implemented API and behavior**

- `scripta help run|check|fmt` and command-local `--help`/`-h` print relevant help to
  stdout with exit 0. Escaped paths and arguments after the source path stay literal.
- `Cli::readLine(maxLineBytes: int optional): string or null` reads buffered UTF-8
  stdin without a prompt or output flush. It defaults to 1 MiB raw lines, including
  terminators, and requires the independent default-enabled `cli.read` capability.
  LF/CRLF, blank lines, final unterminated lines, and repeated EOF follow the file
  stream conventions. Signature/range/permission failures do not consume input.
  Read failures disable further readLine/prompt calls for that registration;
  logical result budgets and cancellation checks apply during bounded assembly.
- The streaming playground accepts `-` for piped data and retains file mode,
  usage errors, and explicit file cleanup.
- `scripta run - [arguments...]` reads source through EOF with UTF-8 and source-budget
  validation. Tags remain required. Includes and packages resolve from the working
  directory, diagnostics use `<stdin>`, and subsequent script input observes EOF.
  `scripta run -- -` continues to address a literal filename.

See the [input contract](standard-library.md#script-arguments-and-prompts) and
[README pipeline examples](../README.md#shell-pipelines). The default library now
exports 105 functions. Input reads are synchronous; cancellation waits for a
blocked read to return. Source-from-stdin is consumed before runtime execution.

**Validation:** On macOS ARM64, `cargo test --workspace --all-features --locked
--offline` passed, including four new CLI pipeline/help tests and five new input
integration tests. The suite ran outside the sandbox because existing HTTP/TLS
fixtures bind loopback ports. Coverage includes Unicode split across buffers,
blank lines/EOF, shared prompt cursors, raw/default/logical limits, strict inputs,
separate permissions, read failures, mid-read cancellation, failed input state,
stdin-source tags/UTF-8/budget/diagnostics, arguments, relative includes, literal
paths, and streaming file/pipe parity.

`cargo clippy --workspace --all-targets --all-features --locked --offline -- -D
warnings`, `cargo fmt --all -- --check`, `git diff --check`, and the streaming
playground's `scripta fmt --check` passed. Direct CLI help and stdin-source smoke
checks passed. These are local results, not Linux/Windows validation.

Callback transforms, precision formatting, writable streams, and new language
syntax remain separate possible scopes.

**Completion acceptance:** Pending.
