# Release procedure

## Release inputs

Choose a reviewed source commit and a tag consistent with the workspace version.
Use the [release checklist](release-checklist.md) to collect native test results,
package checks, dependency reports, and attribution review. The
[compatibility guide](compatibility.md) defines platform baselines and evidence
requirements. [Historical validation records](release-validation.md) retain
results for their recorded source revisions.

## Release notes

Prepare notes for each release describing:

- Changes visible to users, migration steps, and compatibility limitations.
- The source commit and tag, native run URL, target coverage, and evidence links.
- Package contents, optional features, dependency notices, and installation steps.
- Signing or notarization details established by the selected artifacts.
- For promotion, the source candidate and run, unchanged archive hashes, and
  regenerated release metadata.

Review notes against the selected source and artifacts before publication.
Keep earlier receipts and archive identities intact when adding new evidence.

## Command and source filename migration

New artifacts use `scripta`/`scripta.exe` as the primary executable and ship an
identical `scr`/`scr.exe` compatibility copy. `.scripta` is the official source
extension, `.scta` is accepted as shorthand, and legacy `.scr` scripts remain
valid. Opening tags are optional; `<?scripta` and legacy `<?scr` stay accepted.
The formatter preserves either spelling and leaves tag-free source tag-free.
Rust crate names, `SCR_*` environment variables, and the separate playground's `scr-playground` / `--scr` launch
protocol are unchanged. The launcher passes the actual current interpreter path,
so the companion can execute the renamed binary without a protocol change;
its editor file-extension filters belong to the separate companion repository.

Existing native receipts and immutable archives retain their original names and
hashes. Record migration results against the source and packages actually tested.

## Automatic publication on tag pushes

To prepare a new version locally, run from the workspace root:

```sh
python3 scripts/prepare_release.py
# With all Cargo dependencies cached:
python3 scripts/prepare_release.py --offline
```

The script uses the pinned toolchain and its native host target. It runs formatting,
Python tooling tests, collection tests, default/all-feature workspace tests,
minimal-build SQLite tests, Clippy with warnings denied, and `git diff --check`.
After every check succeeds, it prompts for **patch**, **minor**, **major**, or
**cancel** (the default). From 0.1.0 these choices produce 0.1.1, 0.2.0, or 1.0.0.
It updates `Cargo.toml` and the workspace package entries in `Cargo.lock`, preserves
dependency pins and LF/CRLF line endings, and verifies the lockfile with a locked
Cargo resolve. Failed lockfile
validation restores the original version files. A failed test or cancellation
does not bump the version. Existing working-tree edits are allowed; review the
complete intended source before committing.

The script does not commit, tag, push, build local distribution packages, or
publish. Review the version changes and update release notes, commit the intended
source, and push your branch. Then run `python3 scripts/github_release.py` to
validate all four platforms in Actions before creating a tag. GitHub CLI must be
authenticated and the updated workflow must be present on the default branch.
The working tree must be clean and the remote branch must match the local commit.
After successful validation, type the matching tag when prompted to create and
push it on the validated commit. Enter cancels. Failed validation offers at most
two explicit retries per invocation for temporary failures; code fixes need a new
commit and validation. Resume an interrupted run with `--run-id RUN_ID`; the
script verifies its repository, workflow, event, commit, and all four job results.
Use `--remote NAME` to select a remote other than `origin`.

Manual validation creates Actions artifacts without publishing a GitHub Release.
The script does not commit or push your branch. A failed tag push leaves any newly
created local tag available for inspection or retry, and existing remote tags are
never replaced. Pushing the confirmed tag triggers the workflow below, which
rebuilds packages and repeats validation on the bumped version,
runs dist-profile tests, and creates the packages in **GitHub Releases**. Local
checks establish results only for the current host; other native platforms remain
pending until their workflow jobs pass. Python 3.8 or newer, Git, Rustup, and a C
compiler for bundled SQLite are required.

The [tag workflow](development.md#tag-triggered-builds-and-releases) builds Linux
x86-64/ARM64, Windows x86-64, and macOS 15 Apple Silicon. When all native tests
and package checks pass, it publishes downloadable archives and evidence to a
GitHub Release for that tag. Validation and RC tags publish prereleases; the
exact `v<workspace-version>` tag publishes a normal release. Version-shaped tags
must match the workspace's base version. Push the intended reviewed commit's tag;
branch pushes do not publish anything.

The [publisher](../scripts/publish_release.py) validates the downloaded native
records and hashes before staging assets. Standard archive names are unchanged.
Each platform has one standard archive with SQLite included. Publish only four
platform packages, `SHA256SUMS`, and `validation-evidence.zip`: six uploaded
assets. `SHA256SUMS` verifies the four packages and the evidence ZIP. Individual
checksums, native/smoke records, per-target logs, and `release-manifest.json` live
inside the evidence ZIP. Do not upload these supporting files separately.
Standard-package records and extracted-package smoke must both confirm SQLite
is enabled; minimal source builds are tested but are not release variants.

Uploads remain in a draft until every asset succeeds. A failed upload can be
retried while the release is still a draft; a published release is not replaced.
Release downloads do not use the seven-day retention of temporary Actions
artifacts. Existing tags are not republished automatically. Record native results
against the selected source and packages; workflow configuration alone does not
establish platform support.

## Promote validated packages without rebuilding

Use [Promote validated release](../.github/workflows/promote-release.yml) from
the repository's default branch after reviewing a successful candidate. Open
Actions, select that workflow, and choose **Run workflow** on the default branch.
Supply the reviewed candidate and destination:

| Input | Meaning |
| --- | --- |
| `validation_tag` | Published candidate tag whose packages will be reused |
| `validation_run_id` | Successful native build and publication run for that candidate |
| `release_tag` | Stable tag matching `v<workspace-version>` at the validated commit |

The workflow must first be installed on the default branch. Do not create or
push the stable tag yourself: a normal tag push starts the native build workflow.
Promotion creates the stable tag with the workflow's built-in `GITHUB_TOKEN`;
[GitHub does not trigger another workflow for that token's tag event](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/trigger-a-workflow#triggering-a-workflow-from-a-workflow).
Do not substitute a personal access token.

The [promotion verifier](../scripts/promote_release.py) downloads the published
candidate's six assets, which remain available after Actions artifacts expire.
It checks the candidate tag, commit, workspace version, successful native run
and all five jobs, archive hashes, native records, smoke records, and evidence
inventory before creating anything remotely. The stable tag points to the
validated commit, even when the promotion tools are newer on the default branch.

All four platform archives are copied byte-for-byte. Stable release metadata,
`SHA256SUMS`, and the evidence ZIP are regenerated; the manifest records the
source candidate, run, and source evidence hash. Native records and logs retain
their original bytes. There are still exactly six published assets. No Rust
build or package rebuild runs during promotion.

Published releases and existing unrelated tags are never replaced or moved.
After a failed upload, rerun with the same inputs to resume the draft and the
tag created by this promotion. Review and resolve other failures before retrying.

For read-only preparation locally, use a fresh output directory and omit
`--publish`. Set `CANDIDATE_TAG`, `NATIVE_RUN_ID`, and `STABLE_TAG` to the
reviewed inputs:

```sh
python3 scripts/promote_release.py --repository trafficinc/scripta \
  --validation-tag "$CANDIDATE_TAG" \
  --validation-run-id "$NATIVE_RUN_ID" --release-tag "$STABLE_TAG" \
  --output dist/promotion-preview
```

This downloads and verifies assets and prepares the stable files locally; it
does not create a tag or release. Publishing requires manual dispatch on the
default branch with its workflow token.

## Pinned inputs and profiles

Use the compiler pinned in [`rust-toolchain.toml`](../rust-toolchain.toml),
retain the committed `Cargo.lock`, and use `--locked` for release commands. Record the native build OS, SDK/linker, and
system-library versions with validation results.

`release` uses opt-level 3, thin LTO, one codegen unit, no incremental compilation,
checked overflow, and panic unwinding. `dist` inherits release and strips symbols.
Unwinding is intentional: owned child/resource cleanup and the process monitor's
panic containment must keep working. Do not switch to panic=abort to save bytes.
Windows uses `+crt-static`; do not override RUSTFLAGS without retaining it.

Builds embed the commit supplied explicitly by SCR_BUILD_COMMIT. A build without
an explicit commit reports `unknown`, rather than implying a clean/reproducible source revision.
`scripta version` retains the simple version line; `scripta version --verbose` adds target,
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
# Set RELEASE_VERSION to the workspace version.
# macOS ARM64 example; do not use a foreign-target binary as native proof.
export MACOSX_DEPLOYMENT_TARGET=15.0
cargo test --workspace --locked --target aarch64-apple-darwin
cargo test --workspace --all-features --locked --target aarch64-apple-darwin
cargo test --workspace --profile dist --target aarch64-apple-darwin --locked
cargo build -p scr-cli --profile dist --target aarch64-apple-darwin --locked
cargo run -p scr-release --locked --target aarch64-apple-darwin -- package --binary target/aarch64-apple-darwin/dist/scripta --target aarch64-apple-darwin --out-dir dist/local-standard
cargo run -p scr-release --locked --target aarch64-apple-darwin -- verify --archive dist/local-standard/scripta-${RELEASE_VERSION}-aarch64-apple-darwin.tar.gz
cargo run -p scr-release --locked --target aarch64-apple-darwin -- smoke --archive dist/local-standard/scripta-${RELEASE_VERSION}-aarch64-apple-darwin.tar.gz
cargo run -p scr-release --locked --target aarch64-apple-darwin -- checksums --dir dist/local-standard
cargo run -p scr-release --locked --target aarch64-apple-darwin -- verify-checksums --dir dist/local-standard
```

Use the Windows target explicitly for **all** Cargo commands on Windows. Package
extension is selected by the target; target values outside the four configured
candidate names are rejected. `--tag "v${RELEASE_VERSION}"` checks tag/version agreement.
Choose a fresh output directory for each validation run so prior evidence and
archives are retained; `dist/local-standard` above is only an example. Packages
include reference documentation, examples, and runnable playgrounds with their
fixtures. Offline smoke executes the collection playground as well as the
basic examples, and records workspace tests as a separate native-evidence gate.
`--epoch NUMBER` overrides the archive epoch for reproducibility testing. The
maintainer-only scr-release tool requires Rust and cached/fetched crate metadata;
it is not included in or required by the installed scripta interpreter.

The CLI and packager include SQLite by default. `--features sqlite` remains an
explicit equivalent for the packager. For a custom minimal source build, use
`--no-default-features` for the CLI and `--features none` for `scr-release package`.
The packager selects the matching notice graph with default features disabled
and then SQLite explicitly enabled where required. Smoke verifies the executable's
extension list and executes the SQLite example for standard archives. Minimal
custom archives are not published as official release variants. See the
[SQLite build instructions](sqlite.md#validation-and-release-builds).

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

## Limitations

- Collect native evidence before making platform support claims, including for
  other OS baselines, architectures, or device-specific environments. A cross
  build does not replace native tests.
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
  and require --apply; legacy organize.scripta should be read before running.
- Backups are non-overwriting file-tree copies, not atomic snapshots, archives,
  metadata/permission backups, or a disaster-recovery product. Concurrent source
  changes and partial output on failure are possible; symlinks are skipped.

See [installation/uninstallation](install.md), [standard library](standard-library.md),
[examples](../examples/README.md), and the [release checklist](release-checklist.md).
