# Release procedure

## Current candidate

Workspace version: **0.1.0**. This is a first public-test candidate, not a stable
language/API compatibility guarantee. See [compatibility](compatibility.md) for
actual native evidence. No release is automatically published by these changes.

## Pinned inputs and profiles

Rust 1.99.1 is pinned in rust-toolchain.toml; Cargo.lock is committed and every
release command uses `--locked`. Actions are pinned to reviewed full commit IDs.
The native runners are explicit OS images, not `*-latest`; hosted image revisions,
SDKs/linkers, and system libraries can still change. Record their CI run metadata.

`release` uses opt-level 3, thin LTO, one codegen unit, no incremental compilation,
checked overflow, and panic unwinding. `dist` inherits release and strips symbols.
Unwinding is intentional: owned child/resource cleanup and the process monitor's
panic containment must keep working. Do not switch to panic=abort to save bytes.
Windows uses `+crt-static`; do not override RUSTFLAGS without retaining it.

The workflow remaps the checkout path, embeds the commit supplied explicitly by
SCR_BUILD_COMMIT, and normalizes archive metadata. A local build without an explicit
commit reports `unknown`, rather than implying a clean/reproducible source revision.
`scr version` retains the simple version line; `scr version --verbose` adds target,
Cargo's base profile, compiler, explicit commit metadata, and built-in extensions.

Archives sort file entries, normalize permissions/ownership, and use a numeric
SOURCE_DATE_EPOCH (the commit timestamp in CI; default 0 locally). Gzip header
mtime and TAR mtimes are normalized. ZIP uses the fixed representable DOS epoch
1980-01-01 and stored entries. Packaging identical input bytes twice must produce
identical archives; this is tested for both formats. Bit-for-bit compiler output
across different SDK/linker/host images is not promised. Rust profile details are
in the [Cargo profile reference](https://doc.rust-lang.org/cargo/reference/profiles.html).

## Native CI and artifact gate

`.github/workflows/ci.yml` runs format checks, Clippy, and the complete debug suite
on all five native targets, plus a declared-MSRV source compilation job.
`.github/workflows/release.yml` is started manually or by a `v*` tag. It requires
that CI workflow, then tests the complete workspace with the dist profile and
builds scr on each matching native target. No Docker, emulator, Python, or Node
setup is needed by the project commands; GitHub's own Actions infrastructure is
separate from installed interpreter dependencies.

Each target packages docs/examples/licenses, verifies the inventory, extracts and
runs the executable with a cleared environment/PATH containing only the package,
and records `*.validation.json`. Local smoke includes version/target, hello,
variables, loops, functions, require_once, JSON, file reads/writes, dates, and
logging. The full suites additionally test filesystem policies, HTTP/TLS fixtures,
processes/jobs/cancellation, metrics, formatter, parser robustness, and CLI behavior.
Linux/macOS dependency inspection rejects missing or non-system libraries;
Windows import inspection rejects a separate MSVC runtime dependency.

An aggregate artifact is produced only when all five native package jobs succeed.
It includes archives, per-archive `.sha256`, `SHA256SUMS`, per-target validation
records, and dependency reports. Workflow dispatch and tags produce candidates;
publication remains a separate maintainer action after checklist review. The
workflow has read-only repository permissions and never creates/pushes tags,
commits, GitHub Releases, or package-registry uploads.

## Local native package procedure

From a source checkout after the complete tests pass:

```sh
# macOS ARM64 example; do not use a foreign-target binary as native proof.
export MACOSX_DEPLOYMENT_TARGET=15.0
cargo test --workspace --locked
cargo test --workspace --profile dist --target aarch64-apple-darwin --locked
cargo build -p scr-cli --profile dist --target aarch64-apple-darwin --locked
cargo run -p scr-release --locked -- package --binary target/aarch64-apple-darwin/dist/scr --target aarch64-apple-darwin --out-dir dist
cargo run -p scr-release --locked -- verify --archive dist/scripta-0.1.0-aarch64-apple-darwin.tar.gz
cargo run -p scr-release --locked -- smoke --archive dist/scripta-0.1.0-aarch64-apple-darwin.tar.gz
cargo run -p scr-release --locked -- checksums --dir dist
cargo run -p scr-release --locked -- verify-checksums --dir dist
```

Use the Windows target explicitly for **all** Cargo commands on Windows. Package
extension is selected by the target; target values outside the five configured
candidate names are rejected. `--tag v0.1.0` validates tag/version agreement.
`--epoch NUMBER` overrides the archive epoch for reproducibility testing. The
maintainer-only scr-release tool requires Rust and cached/fetched crate metadata;
it is not included in or required by the installed scr interpreter.

For a SQLite-enabled variant, build the CLI with `--features sqlite` and pass
`--features sqlite` to the `scr-release package` subcommand as well. This selects
the matching dependency notices and records the extension in `release.json`.
Smoke checks the executable's reported extension list and runs the SQLite example
for enabled archives. Use a separate output directory for each variant; archive
names do not encode features. See the [SQLite release instructions](sqlite.md#validation-and-release-builds).

SHA-256 sidecars contain `hex  filename` in the standard checksum format.
`checksums --dir` writes a sorted manifest for every archive in that directory;
`verify-checksums --dir` rejects altered, missing, or extra archives relative to it.
The package manifest inventories every payload file, including dependency notices.
Version-specific supplements under release-notices preserve upstream declarations
from exact crate revisions plus canonical SPDX terms when crate archives omit
license files. Provenance/hash checks are mandatory and unknown versions fail;
publication still requires attribution/license review.
Verification rejects unsafe paths, duplicates, non-regular TAR entries, missing
files, and hash mismatches, with a 128 MiB/4096-file verification budget.
Checksums provide integrity, not publisher authentication.

## Limitations for 0.1.0

- Only rows with recorded native evidence may be called verified. Older OS releases,
  other architectures, ARM32, musl, and device-specific Raspberry Pi behavior are
  unverified. A successful cross build is not a substitute for running the tests.
- Candidates are not notarized/Developer-ID-signed or Authenticode-signed. No MSI,
  PKG, package-manager publication, automatic updater, or privileged installer is
  provided. Platform security prompts may require manual review.
- Scripta is dynamically typed and not PHP-compatible. Interpolation, script-defined
  classes/Exception construction, package management, a VM/JIT, and background
  daemons remain absent. Check validates syntax/declarations, not complete types.
- Filesystem root checks are not an OS sandbox or race-proof defense against a
  hostile directory tree. The interpreter executes trusted automation code with
  the user's permissions; Process execution capabilities are broad.
- Local source defaults to 8 MiB, runtime value nesting to 128, calls/includes to
  at most 64. Configurable logical-value/scope/step budgets are not an aggregate
  RSS ceiling. Captured scopes remain until runtime drop; see the safety audit.
- Whole-file UTF-8 reads and value-copy semantics can use substantial memory.
  Subprocess capture and HTTP bodies have independent default 8 MiB ceilings.
  Temporary capture files can overshoot between monitor polls. HTTP cancellation
  waits for blocking operations/timeouts; no custom CA or binary body API exists.
- Native metric capabilities may be unavailable on restricted systems and then
  raise explicit errors. CPU usage blocks for a real interval; memory is system
  memory. No instantaneous CPU values or process-memory/system-memory substitution.
- Scripts that explicitly use Http need their endpoint; scripts using Process need
  the requested executable. Those optional actions do not create a runtime or
  network dependency for local scripts. Examples with file changes preview first
  and require --apply; legacy organize.scr should be read before running.
- Backups are non-overwriting file-tree copies, not atomic snapshots, archives,
  metadata/permission backups, or a disaster-recovery product. Concurrent source
  changes and partial output on failure are possible; symlinks are skipped.

See [installation/uninstallation](install.md), [standard library](standard-library.md),
[examples](../examples/README.md), and the [release checklist](release-checklist.md).
