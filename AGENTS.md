# Scripta repository guidance

## Project and references

Scripta is a standalone scripting language implemented in Rust. Work from the
workspace root using `rust-toolchain.toml`; preserve the declared minimum Rust
version and `unsafe_code = "forbid"` policy.

Read relevant implementation and tests before changing behavior. Keep affected
references aligned: `docs/language-spec.md` (semantics), `docs/standard-library.md`
(APIs/policies), `docs/architecture.md` (design), `docs/development.md` (workflows),
and `docs/writing-extensions.md` (native integration).

Preserve unrelated working-tree edits. Do not commit, publish or release unless
requested. Keep temporary task status out of this file.

## Code ownership

All crates below live in `crates/`:

- `scr-ast`, `scr-lexer`, `scr-parser`, `scr-diagnostics`: syntax and diagnostics.
- `scr-runtime`: values, scopes, evaluation, objects, native interfaces and limits;
  keep standard-library backends out of it.
- `scr-stdlib`: automation APIs and host policies.
- `scr-db`: shared database API and runtime-owned handles.
- `scr-ext-sqlite`, `scr-ext-mysql`, `scr-ext-postgres`: database backends;
  CLI SQLite is default-enabled, MySQL/PostgreSQL optional.
- `scr-web`: HTTP supervision and interpreter workers.
- `scr-cli`: commands, formatting, packages and executable integration.
- `scr-release`: packaging and dependency notices.

Runnable scripts live in `examples/` and `playground/`; shared fixtures in
`tests/fixtures/`.

## Implementation rules

- Preserve strict types and explicit conversions: UTF-8 strings, binary bytes,
  Unicode-scalar string positions, and explicit grapheme operations.
- Use public native registration. Validate signatures, options, ranges, resource
  ownership and closed state. Reuse filesystem policies/capabilities; preserve
  TLS verification, HTTP ceilings and overwrite/symlink rules.
- Bound growing outputs before allocation where practical, accounting for byte
  payloads and collection slots. Logical value limits are not a heap cap.
- Check cancellation during iterative native work. Use RAII; cancellation and
  fatal limits may bypass script cleanup.
- Preserve error kinds, spans, traces and failed-assignment behavior. Never expose
  credentials in errors.
- Follow existing patterns and keep changes focused. Justify dependencies,
  preserve minimum Rust compatibility and update Cargo.lock intentionally.
- Update affected API references, export-reference checks and runnable examples
  when public behavior changes.
- Preserve `.gitattributes` LF rules for build/release inputs, including `*.py`
  and Cargo.lock, plus byte-exact fixture exceptions. Text-processing scripts must
  handle LF/CRLF; preserve existing endings in targeted edits and test both forms
  with explicit byte fixtures.

## Validation

Keep validation guidance here. Choose checks by the behavior changed:

- **Documentation:** check links/paths and `git diff --check`; generally no Rust tests.
- **Code:** start with affected crate/integration tests. Cover meaningful behavior,
  failures and boundaries, not tests that merely mirror implementation.
- **Cross-crate, public API, dependency or feature changes:** after focused tests,
  run formatting, full default/all-feature tests and all-target/all-feature Clippy.
- **Releases:** additionally complete the native evidence requirements below.

Use the native `TARGET` from the platform table for build, test and Clippy commands.
Verify `rustc -vV` reports that host; cross-compilation is not native validation.
Explicit targets retain Windows static-CRT settings in `.cargo/config.toml`.
Set `MACOSX_DEPLOYMENT_TARGET=15.0` for macOS builds. SQLite requires a C compiler.

```sh
cargo test -p scr-runtime --locked --target TARGET
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked --target TARGET -- -D warnings
cargo test --workspace --locked --target TARGET
cargo test --workspace --all-features --locked --target TARGET
cargo test -p scr-stdlib --lib collections --locked --target TARGET
cargo test -p scr-stdlib --test collections --locked --target TARGET
git diff --check
```

Use `cargo fmt --all` to apply formatting. Test default features separately when
optional features change. Use `--offline` for cached dependencies, not intentional
first fetches. After merges, rebases or version bumps, run `cargo metadata --locked`;
correct stale workspace versions without changing dependency pins. Retain `--locked`
in CI.

Unit tests live in crate source modules; integration targets in each crate's
`tests/`. Root `tests/language/` and `tests/integration/` are organizational only.
Runtime tests protect type/error semantics, limits, memory, functions and cleanup;
stdlib tests cover host/data APIs; CLI tests cover execution, diagnostics, formatting,
packages, extensions and examples.

Use temporary/local fixtures, never public HTTP endpoints. Socket/process tests
may require sandbox approval: report restrictions and use the approval mechanism.
Never weaken/skip assertions to bypass sandbox or platform failures. Preserve
platform guards. Resource changes need exact/plus-one limits, cancellation,
cleanup and unchanged destinations after failure where relevant.

For changed `.scripta` examples, run CLI `fmt --check`, `check` and `run` with
controlled inputs via `cargo run -p scr-cli --locked --target TARGET -- ...`.
Syntax checks do not execute code or validate runtime types.

Report changes, actual checks and limitations. Record revision, compiler, native
OS/architecture and results. Unavailable platforms remain pending; macOS success
proves neither Linux nor Windows. Do not claim unrun checks passed or repeat
passing checks without new changes or unresolved failures.

## Public naming

Use `scripta` and `.scripta` for new commands/scripts; retain `.scta`, `scr` and
`.scr` compatibility. Keep internal `scr-*` crates and existing `SCR_*` names.
New scripts are tag-free; accept optional `<?scripta` and legacy `<?scr`, preserving
opening spellings or tag absence when formatting. New manifests declare
`main.scripta`; omitted legacy entries resolve to `main.scr`. Preserve historical
validation receipts and hashes when renaming.

## Native platforms and releases

Do not create GitHub Actions workflows without explicit instruction; leave
existing triggers unchanged. Validate natively on these baselines when available:

| Native host | Rust target |
| --- | --- |
| Ubuntu 22.04 x86-64 | `x86_64-unknown-linux-gnu` |
| macOS 15 ARM64 | `aarch64-apple-darwin` |
| Windows Server 2022 x86-64 | `x86_64-pc-windows-msvc` |

Linux ARM64 (`aarch64-unknown-linux-gnu`) is source-only: retain native source
builds and local test launchers, but do not require ARM64 jobs for release or
publish ARM64 packages.

macOS support is Apple Silicon only: do not add Intel targets or require Intel
validation. Each native audit requires the validation checks above, dist-profile
tests, dependency inspection, archive verification and extracted-package smoke.
Debug tests/lints alone do not establish release support. Update
`docs/compatibility.md` only with the required native evidence.

Build all three release platforms and publish only after every native test/package
job succeeds and publication is explicitly requested. Public releases contain
exactly four assets: three SQLite-enabled platform archives and `SHA256SUMS`.
Each archive contains only `scripta`, the compatible `scr` command (with `.exe`
on Windows), required non-system runtime libraries, and license notices. Exclude
docs, examples, playground files, logs, validation records and internal manifests.
Keep reports, smoke results, logs and manifests in local build outputs or internal
CI artifacts, separate from public assets. Do not build or publish an evidence ZIP.
Minimal `--no-default-features` builds remain source-only.

Workspace-version tags publish normal releases; validation/RC/other tags publish
prereleases. Do not replace published assets or move existing tags.

<!-- repogroom:start -->
## Development Guidelines

Read and follow [DEVELOPMENT_GUIDELINES.md](./DEVELOPMENT_GUIDELINES.md)
before creating, modifying, reviewing, or documenting code.
<!-- repogroom:end -->
