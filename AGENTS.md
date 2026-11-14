# Scripta repository guidance

## Project and sources of truth

Scripta is a standalone scripting language implemented in Rust. Run commands
from the workspace root. Use the toolchain in `rust-toolchain.toml`; preserve the
workspace's declared minimum Rust version and `unsafe_code = "forbid"` policy.

Read the relevant existing implementation and tests before changing behavior.
Use these documents for context, keeping them aligned with code changes:

- `docs/language-spec.md`: grammar, types, evaluation, and error semantics.
- `docs/standard-library.md`: exported APIs, permissions, and resource limits.
- `docs/architecture.md`: crate boundaries and runtime design.
- `docs/development.md`: development and validation workflows.
- `docs/writing-extensions.md`: native registration and extension patterns.
- `docs/releases.md` and `docs/release-checklist.md`: packaging and releases.

## Code ownership

- `crates/scr-ast`, `scr-lexer`, `scr-parser`, and `scr-diagnostics`: syntax,
  parsing, source locations, and diagnostics.
- `crates/scr-runtime`: values, evaluation, scopes, callables, native interfaces,
  cancellation, and execution limits. Keep standard-library backends out of it.
- `crates/scr-stdlib`: registered automation APIs and their host policies.
- `crates/scr-cli`: commands, formatting, packages, and executable integration.
- `crates/scr-ext-sqlite`: SQLite implementation; CLI feature `sqlite` is enabled by default.
- `crates/scr-release`: distribution packaging and dependency notices.
- `examples/` and `playground/`: runnable scripts; shared fixtures live in
  `tests/fixtures/`.

## Implementation expectations

- Preserve strict types and explicit conversions. Strings are valid UTF-8;
  `bytes` holds arbitrary binary data. Existing string length and substring
  positions count Unicode scalars; grapheme operations are explicit.
- Use the public native registration interface for standard-library features.
  Validate signatures, options, ranges, resource ownership, and closed state.
- Reuse filesystem policies and capability checks. Preserve TLS verification,
  HTTP ceilings, and existing overwrite/symlink rules.
- Bound growing outputs before allocation where practical. Account for byte
  payloads and collection slots; logical value limits are not a process heap cap.
- Check cancellation during iterative native work. Use Rust RAII for cleanup;
  script cleanup may be bypassed on cancellation or fatal execution limits.
- Preserve error kinds, source spans, stack traces, and failed-assignment
  behavior. Keep error messages useful without exposing credentials.
- Follow existing patterns; keep changes focused. Justify new dependencies and
  preserve minimum Rust compatibility. Update `Cargo.lock` intentionally.
- Update affected API references, export-reference checks, and runnable examples
  when public behavior changes. Syntax checks alone do not prove runtime behavior.
- Preserve unrelated working-tree edits. Do not commit, publish, or perform a
  release unless requested. Avoid recording temporary task status in this file.

## Tests and validation

Choose tests by the behavior changed. Add regression tests for meaningful bugs,
new behavior, failure handling, and boundaries; avoid tests that only repeat the
implementation. Documentation-only changes generally need link/path checks and
`git diff --check`, not a full Rust test run.

Start with the affected crate or integration target, for example:

```sh
cargo test -p scr-runtime --locked
cargo test -p scr-parser --test robustness --locked
cargo test -p scr-stdlib --test data_handling --locked
cargo test -p scr-stdlib --test filesystem --locked
cargo test -p scr-stdlib --test json_http_modules --locked
cargo test -p scr-cli --test release_examples --features sqlite --locked
```

Unit tests are in crate source modules. Cargo integration tests are in each
crate's `tests/` directory. Root `tests/language/` and `tests/integration/` are
organizational documentation, not automatically discovered Cargo test targets.

For code changes that cross crates, alter public APIs, change dependencies, or
affect feature wiring, run the full checks after focused tests pass:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
git diff --check
```

Use `cargo fmt --all` to apply Rust formatting. Keep default-feature behavior
working too; test it separately when optional-feature behavior changes. SQLite
builds require a C compiler. Add `--offline` when dependencies are already cached;
do not use it for an intentional first-time dependency fetch.

Important test coverage and environment details:

- `scr-runtime/tests`: type/error semantics, resource limits, memory lifecycle,
  functions, and cleanup. Runtime changes should preserve these invariants.
- `scr-stdlib/tests`: filesystem, process, HTTP/TLS, streams, CSV, bytes, regex,
  and Unicode behavior. Use temporary directories and local fixtures.
- `scr-cli/tests`: complete language execution, exit codes, diagnostics,
  formatting snapshots, packages, optional extensions, and release examples.
- HTTP tests bind localhost sockets; process tests spawn fixture children.
  Sandbox restrictions can block these operations. Report the restriction and
  use the environment's approval mechanism when needed; do not weaken or skip
  assertions just to make sandbox failures pass.
- Tests should not depend on public HTTP endpoints. Preserve platform guards;
  successful macOS validation does not establish Windows or Linux correctness.
- For resource changes, cover exact limits and limit-plus-one, cancellation,
  cleanup, and unchanged destinations after failure where relevant.

For changed `.scripta` examples, check formatting and syntax, then execute them with
controlled inputs. `scripta check` does not execute code or validate runtime types:

```sh
cargo run -p scr-cli --locked -- check examples/data-handling.scripta
cargo run -p scr-cli --locked -- fmt --check examples/data-handling.scripta
cargo run -p scr-cli --locked -- run examples/data-handling.scripta
```

## Public naming

Use `scripta` as the primary command and `.scripta` for new source files. `.scta`
is a shorthand; retain the `scr` command and `.scr` file compatibility. Keep
internal `scr-*` crate names and existing `SCR_*` configuration names. Use
`<?scripta` for new source, keep legacy `<?scr` accepted, and preserve the
existing opening spelling when formatting.
New project manifests explicitly declare `main.scripta`; omitted entries in
legacy manifests still resolve to `main.scr`. Preserve historical validation
receipts and hashes when updating names.

## Cross-platform validation

Keep validation guidance in this file. Do not create GitHub Actions workflows
unless the user explicitly requests them.
The authorized release workflow runs on every tag push, with no branch/PR
triggers. Build Linux x86-64/ARM64, Windows x86-64, and macOS 15 Apple Silicon;
publish release downloads only after every native test/package job succeeds.
Keep release downloads compact: exactly four platform packages, all with SQLite,
`SHA256SUMS`, and one `validation-evidence.zip` (six uploaded assets). Keep minimal
`--no-default-features` builds available from source, not as separate downloads. Bundle
individual checksums, native/smoke records, logs, and the release manifest inside
the evidence ZIP; do not upload these as separate release assets. Version tags matching the workspace
publish normal releases; validation/RC/other tags publish prereleases. Do not
replace already published assets or move existing tags.

macOS support is limited to Apple Silicon (M-series) Macs. Do not add Intel
macOS release targets or require Intel validation.

Validate natively on the candidate platform baselines when hosts are available:

| Native host | Rust target |
| --- | --- |
| Ubuntu 22.04 x86-64 | `x86_64-unknown-linux-gnu` |
| Ubuntu 22.04 ARM64 | `aarch64-unknown-linux-gnu` |
| macOS 15 ARM64 | `aarch64-apple-darwin` |
| Windows Server 2022 x86-64 | `x86_64-pc-windows-msvc` |

- Use the pinned toolchain and verify that the compiler host from `rustc -vV`
  matches the target. Cross-compilation is not native validation.
- Pass an explicit `--target` to build, test, and Clippy commands, especially on
  Windows to retain `.cargo/config.toml` static-CRT settings. For macOS builds,
  set `MACOSX_DEPLOYMENT_TARGET=15.0`.
- Run focused collection resource-limit tests (`cargo test -p scr-stdlib --lib
  collections --locked --target TARGET`) and collection behavior tests
  (`cargo test -p scr-stdlib --test collections --locked --target TARGET`),
  replacing TARGET with the native triple above.
- Run formatting, full workspace tests with default and all features, and
  all-target/all-feature Clippy with warnings denied. Do not suppress platform
  failures or weaken assertions to make validation pass.
- Record the tested revision, compiler, native OS/architecture, and results.
  When native hosts are unavailable, report validation as pending; successful
  local macOS tests do not establish Linux or Windows correctness.
- Debug tests and lint checks do not establish release support. Follow
  `docs/releases.md` and `docs/release-checklist.md` for dist-profile tests,
  dependency inspection, archive verification, and extracted-package smoke.
  Update `docs/compatibility.md` only with the required native evidence.

Report what changed, the checks actually run, and any remaining limitations.
Do not claim unrun tests or untested platforms passed. Once relevant checks pass,
repeat them only when new changes or unresolved failures justify another run.
