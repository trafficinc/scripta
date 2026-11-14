# Release procedure

## Current candidate

Workspace version: **0.1.0**. This is a first public-test candidate, not a stable
language/API compatibility guarantee. See [compatibility](compatibility.md) for
actual native evidence. No release is automatically published by these changes.
See the [validation record](release-validation.md) for the reviewed scope,
source provenance, generated evidence, and outstanding release gates.

## Draft candidate notes

Scripta 0.1.0 is a first public-test candidate. These notes are prepared for
maintainer review; they do not announce a published release or stable API.

- Strict scripting with explicit conversions, UTF-8 strings and immutable bytes,
  separate lists/maps, lexical functions/closures, and tracing of unreachable
  captured scopes. Cached module roots and host-held callables remain protected.
- Standard-library automation covers filesystem operations, bounded subprocesses,
  HTTP/HTTPS with text or binary bodies, JSON, CSV, regular expressions, dates,
  logging, system metrics, and optional SQLite. The SQLite extension does not
  currently bind or return the core bytes type as BLOB data.
- Collection helpers provide list end operations and splice, generation,
  aggregation, comparisons, record columns, and typed map merging/construction.
  Key-based map operations use temporary indexes while preserving entry order.
- Git/local package installation, isolated aliases, syntax checks, diagnostics,
  and formatting are implemented. Interpolation, script-defined classes,
  callback-based collection transforms, and recursive merge variants are absent.
- The local macOS 27.0.1 ARM64 run passed 422 default-feature debug tests,
  423 all-feature debug tests, and 423 all-feature dist tests. Rust 1.85 source
  compatibility, formatting, Clippy, both archive variants, and offline package
  smoke passed. See the [validation record](release-validation.md) for scope.
- Base and SQLite archives were reviewed with all 34 shipped scripts checked,
  isolated local installation/removal, and matching dependency-notice inventories.
  These checks establish local artifact behavior, not other-platform support or
  maintainer attribution signoff.
- Native Ubuntu 22.04 x86-64 and ARM64 validation passed on
  `v0.1.0-validation.20261113.6`: 433 default-feature and 434 all-feature tests
  in each of debug and dist profiles, with no failed or ignored tests. Clippy,
  base/SQLite package verification, dependency inspection, offline smoke, and
  identical-input repacking passed; see the
  [Actions evidence](release-validation.md#native-github-actions-validation).
- Native Windows Server 2022 x86-64 validation passed on the same tag:
  426 default-feature and 427 all-feature tests in each profile, with no failed
  or ignored tests. Both package variants passed verification, static-CRT import
  inspection, offline smoke, and identical-input repacking.
- The binaries target macOS 15.0 but have not been run on that OS baseline.
  macOS support is limited to Apple Silicon (M-series) Macs; Intel Macs are
  excluded. macOS 15 ARM64 native evidence remains pending.
  Reviewed macOS binaries have linker-generated ad hoc signatures, with no
  publisher identity or notarization. No Windows signing is claimed.

Before publication, review attribution notices and changes, select a clean
source revision and matching tag, collect native evidence for the selected
platforms, and repackage that final source. The reviewed local archives predate
these final documentation-review updates; their hashes are retained separately
and no current archive is presented as a clean tagged release.

## Pinned inputs and profiles

Rust 1.99.1 is pinned in rust-toolchain.toml; Cargo.lock is committed and every
release command uses `--locked`. Record the native build OS, SDK/linker, and
system-library versions with validation results.

`release` uses opt-level 3, thin LTO, one codegen unit, no incremental compilation,
checked overflow, and panic unwinding. `dist` inherits release and strips symbols.
Unwinding is intentional: owned child/resource cleanup and the process monitor's
panic containment must keep working. Do not switch to panic=abort to save bytes.
Windows uses `+crt-static`; do not override RUSTFLAGS without retaining it.

Builds embed the commit supplied explicitly by SCR_BUILD_COMMIT. A build without
an explicit commit reports `unknown`, rather than implying a clean/reproducible source revision.
`scr version` retains the simple version line; `scr version --verbose` adds target,
Cargo's base profile, compiler, explicit commit metadata, and built-in extensions.

Archives sort file entries, normalize permissions/ownership, and use a numeric
SOURCE_DATE_EPOCH (default 0; use the commit timestamp when packaging a release).
Gzip header mtime and TAR mtimes are normalized. ZIP uses the fixed representable DOS epoch
1980-01-01 and stored entries. Packaging identical input bytes twice must produce
identical archives; this is tested for both formats. Bit-for-bit compiler output
across different SDK/linker/host images is not promised. Rust profile details are
in the [Cargo profile reference](https://doc.rust-lang.org/cargo/reference/profiles.html).

## Local native package procedure

From a source checkout after the complete tests pass:

```sh
# macOS ARM64 example; do not use a foreign-target binary as native proof.
export MACOSX_DEPLOYMENT_TARGET=15.0
cargo test --workspace --locked --target aarch64-apple-darwin
cargo test --workspace --all-features --locked --target aarch64-apple-darwin
cargo test --workspace --profile dist --target aarch64-apple-darwin --locked
cargo build -p scr-cli --profile dist --target aarch64-apple-darwin --locked
cargo run -p scr-release --locked --target aarch64-apple-darwin -- package --binary target/aarch64-apple-darwin/dist/scr --target aarch64-apple-darwin --out-dir dist/local-base
cargo run -p scr-release --locked --target aarch64-apple-darwin -- verify --archive dist/local-base/scripta-0.1.0-aarch64-apple-darwin.tar.gz
cargo run -p scr-release --locked --target aarch64-apple-darwin -- smoke --archive dist/local-base/scripta-0.1.0-aarch64-apple-darwin.tar.gz
cargo run -p scr-release --locked --target aarch64-apple-darwin -- checksums --dir dist/local-base
cargo run -p scr-release --locked --target aarch64-apple-darwin -- verify-checksums --dir dist/local-base
```

Use the Windows target explicitly for **all** Cargo commands on Windows. Package
extension is selected by the target; target values outside the four configured
candidate names are rejected. `--tag v0.1.0` validates tag/version agreement.
Choose a fresh output directory for each validation run so prior evidence and
archives are retained; `dist/local-base` above is only an example. Packages
include reference documentation, examples, and runnable playgrounds with their
fixtures. Offline smoke executes the collection playground as well as the
basic examples, and records workspace tests as a separate native-evidence gate.
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
  PKG, OS package-manager publication, automatic updater, or privileged installer is
  provided. Platform security prompts may require manual review.
  Local macOS binaries have linker-generated ad hoc signatures; these do not
  identify a publisher.
- Scripta is dynamically typed and not PHP-compatible. Interpolation, script-defined
  classes/Exception construction, a VM/JIT, and background
  daemons remain absent. Check validates syntax/declarations, not complete types.
- The built-in [package manager](packages.md) supports Git and local dependencies,
  lockfiles, and offline installation from installed Git snapshots or local sources.
  Git packages require the Git executable. There is no registry lookup, package
  publish command, or install-time script.
- Filesystem root checks are not an OS sandbox or race-proof defense against a
  hostile directory tree. The interpreter executes trusted automation code with
  the user's permissions; Process execution capabilities are broad.
- Local source defaults to 8 MiB, runtime value nesting to 128, calls/includes to
  at most 64. Configurable logical-value/scope/step budgets are not an aggregate
  RSS ceiling. A tracing collector reclaims unreachable captured scopes during
  execution and at execution boundaries. Host-retained callables remain roots;
  opaque native ownership cycles require host cleanup. Freed storage need not
  immediately reduce RSS; see the [safety audit](performance.md).
- Whole-file UTF-8 reads and value-copy semantics can use substantial memory.
  Subprocess capture and HTTP bodies have independent default 8 MiB ceilings.
  Temporary capture files can overshoot between monitor polls. HTTP cancellation
  waits for blocking operations/timeouts. HTTP accepts string/byte request
  bodies and binary responses; downloads stream to bounded temporary files.
  No script-level custom CA option exists.
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
