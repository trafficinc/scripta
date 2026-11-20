# Testing Scripta packages

`scripta test` runs tests written in Scripta using the installed executable. No
Python, test framework package, or Rust compiler is required by package authors.
The interpreter's Rust tests and repository release/benchmark tooling remain
separate development checks.

## Write and run tests

Create `tests/greeting_test.scripta`:

```scripta
require "../src/main.scripta" as Greetings;

function test_greeting() {
    Assert::same("Hello, Ada!", Greetings::hello("Ada"));
}

function test_rejects_wrong_types() {
    Assert::throws(function () {
        return 1 + "1";
    }, "TypeError");
}
```

The `Greetings` API above is illustrative. A runnable version using a local
package is [the greeting test](../examples/packages/tests/greeting_test.scripta).
From `examples/packages`, run `scripta install --locked --offline`, then
`scripta test`.

```sh
scripta test
scripta test tests/greeting_test.scripta
scripta test --filter greeting
scripta test --list
scripta test --fail-fast
scripta test --format json
scripta test --show-output
```

Default discovery recursively reads `tests/**/*_test.scripta`. Top-level named
functions beginning with `test_` are cases and must have no parameters, including
optional/variadic parameters. Return values are ignored; assertions or uncaught
errors determine the result. Helpers are ordinary functions. Nested and included
functions are not discovered as cases. Files under `fixtures`, `.git`, `.scripta`,
and `target` are excluded from directory discovery; symlinks are not followed.
An explicit symlink path is an error. Explicit regular files may use `.scr` or
other existing source spellings. Multiple paths are deduplicated; files and
functions are sorted. `--filter` matches a case-sensitive substring of
`file::function`. Use `--` before dash-prefixed paths.

Discovery parses and validates all selected files before executing any case.
Syntax errors, duplicate declarations, invalid case signatures, or no matching
cases return an error. `--list` executes no initializers, includes, or test bodies.

## Assertions and fixtures

`Assert` and `Test` are runner-owned native namespaces. They are available during
file initialization and test execution, without a `require`; ordinary `scripta
run` and embedded runtimes do not register them.

| API | Contract |
| --- | --- |
| `Assert::same(expected, actual, message optional): null` | Strict type/value equality, including nested collections and bytes; native objects/callables retain existing identity semantics. |
| `Assert::notSame(expected, actual, message optional): null` | Strict inequality. |
| `Assert::isTrue(value, message optional): null` | Requires exactly `true`. |
| `Assert::isFalse(value, message optional): null` | Requires exactly `false`. |
| `Assert::contains(needle, value, message optional): null` | String substring, or strict membership in a list. Other combinations raise `TypeError`. |
| `Assert::throws(callback, kind: string, message optional): null` | Invoke with no arguments and require an exact catchable exception kind. Explicit throws use their runtime type name, e.g. `string`. |
| `Assert::fail(message: string): null` | Record an assertion failure. |
| `Test::skip(reason: string): null` | Stop the case and report it skipped. |
| `Test::tempDir(): string` | Return the current case's existing temporary directory; requires `filesystem.write`. |

Optional assertion messages must be strings. Failed assertions raise
`AssertionError`, with bounded expected/actual diagnostics. Failure is sticky:
catching that exception or expecting it with `Assert::throws` does not turn the
case into a pass. A prior assertion failure takes precedence over a later skip.
`Assert::throws` propagates exits, cancellation, fatal errors, and resource-limit
errors instead of accepting them as an expected exception. Skipping aborts the
case through an interruption signal; script `finally` cleanup is not guaranteed.

Fixtures can use ordinary helper functions and `try`/`finally`. Use
`Test::tempDir()` for files that the runner should remove, including on timeout
and interruption. The directory is unique to the case and owned by the parent
runner. Setup/teardown hooks, data providers, mocks, snapshots, coverage, and
parallel execution are not part of this first version.

## Isolation and limits

Each case gets a fresh process/runtime. Its test file is initialized before the
selected function is invoked, so top-level side effects repeat for each case.
Variables, runtime environment changes, and module caches are independent. The
worker uses the nearest project root as its working directory, or the invoking
directory for standalone tests. Relative `require` paths retain source-relative
resolution. Standard-library capabilities, configured execution limits, and the
executable's available native extensions remain in force. No package installation,
Git operation, or index lookup is triggered by the runner itself.

This is process isolation, not a filesystem or network sandbox. Tests can still
modify shared files or external services. Workers use process-group/job cleanup;
independently detached processes and external resource cleanup are not guaranteed.

Configure optional defaults in `scripta.toml`:

```toml
[test]
directories = ["tests"]
timeout_ms = 10000
max_output_bytes = 1048576
```

Configuration is preserved by package commands. Directories must contain 1–64
relative paths without `..`. Timeouts accept 1–3,600,000 ms. Combined stdout/stderr
capture accepts 0–67,108,864 bytes per case; the default is 1 MiB. Output beyond
the limit fails the case. Capture uses a temporary file and checks its size while
polling the worker; captured report bytes are bounded, but this is not a hard
temporary-disk quota between polling intervals. Timeouts include worker startup
and file initialization. An explicit `exit`, including `exit 0` during either
initialization or the test body, is a failure.

Discovery is bounded to 4,096 files, 4,096 selected cases, 64 directory levels,
64 MiB of total source, 4,096 bytes per function name, and 8 MiB of case
identifiers. The normal per-source
`SCR_MAX_SOURCE_BYTES` ceiling also applies. Existing `SCR_MAX_STEPS`,
`SCR_MAX_VALUE_BYTES`, and `SCR_MAX_SCOPES` settings apply in workers; initialization
and host invocation share the instruction budget. These logical limits do not
constitute an aggregate process heap cap.

## Reports and exit codes

Text output prints `PASS`, `FAIL`, or `SKIP` with each case name and a summary.
Failures include original source locations and stack traces. Captured output is
shown for failures; use `--show-output` to retain/display passing output. Known
environment secrets and credential-shaped text are redacted. Expected/actual
values and diagnostics are truncated when needed; accumulated diagnostic/output
payload is capped at 64 MiB per suite.

JSON reports contain `version: 1`, `tests`, `summary` (`passed`, `failed`,
`skipped`), and `interrupted`. Case records include `file`, `name`, `status`,
`duration_ms`, `diagnostic`, and `output`. A JSON listing contains `version` and
`tests` with `file`/`name`. Configuration/discovery errors contain `version`, an
empty `tests` array, and `error`. Script stdout/stderr never frame worker results;
worker completion uses a separate bounded report file.

| Exit | Meaning |
| --- | --- |
| `0` | All executed tests passed or were skipped; listing succeeded. |
| `1` | One or more cases failed, timed out, exceeded output limits, or had an abnormal worker exit. |
| `2` | Usage, configuration, discovery, or runner infrastructure error; no matching tests. |
| `130` | Runner interrupted. |

`--fail-fast` stops after the first failure and reports only cases that ran.
