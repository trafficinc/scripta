# Release validation — 2026-11-13

This record covers local macOS release-readiness work for Scripta 0.1.0 and
subsequent native Linux/Windows validation. No release has been published.

## Source and environment

- Source base: `55fa03d191568838a94e99951e758941c88517cb`.
- Tested workspace: local, uncommitted documentation and release-tooling changes
  on that base. No tag was created or verified. The interpreter's build revision
  remains `unknown`; the base revision is not presented as a clean release commit.
- Native host: macOS 27.0.1 (26A434), ARM64.
- Target: `aarch64-apple-darwin`; deployment target: macOS 15.0.
- Compiler: Rust 1.99.1; SDK: macOS 27.0; Apple Clang 21.0.0.
- Dependency inputs: committed `Cargo.lock`, `--locked`, offline cached crates.

The declaration of macOS 15.0 as a deployment target is not evidence of execution
on macOS 15. At the time of this local run, the other targets had not been
validated natively. This local run did not use GitHub Actions. Subsequent
[tag-triggered validation](development.md#tag-triggered-builds-and-releases)
collects separate native Linux/Windows evidence.

## Validation scope

Validation runs focused release-tool tests, formatting, all-target/all-feature
Clippy, a Rust 1.85.0 all-target/all-feature source check, native debug workspace
tests with default and all features, and native dist-profile workspace tests
with all features. Optimized base and SQLite-enabled executables are packaged
separately. Archive verification, checksums, identical-input repacking, system
library inspection, and extracted-package offline smoke are checked for both.

| Native test suite | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| Debug workspace, default features | 422 | 0 | 0 |
| Debug workspace, all features | 423 | 0 | 0 |
| Dist workspace, all features | 423 | 0 | 0 |

Formatting, all-target/all-feature Clippy with warnings denied, and the Rust
1.85.0 all-target/all-feature source check passed. These results include the
collection semantics, cancellation and value budgets, closure lifetime/collector
tests, package-manager tests, and optional SQLite integration coverage.

The package smoke runs the shipped examples and collection playground with a
cleared environment and a PATH containing only the extracted package. The
SQLite variant also executes the bundled SQL example. The archive includes the
playgrounds and their fixtures, so README/reference links to runnable examples
remain usable after extraction.

Generated test logs, binary and archive hashes, dependency reports, package smoke
records, and the combined validation record are retained locally under
`dist/readiness-2026-11-13-aarch64-apple-darwin/`. This ignored output directory is
not part of the source distribution. The complete generated record is
`validation.json`; smoke records alone do not establish workspace test results.

## Follow-up archive review

The previously built base and SQLite archives were inspected by their recorded
SHA-256 identities. Both passed isolated installation/removal in a temporary
prefix, example execution from an unrelated working directory, and syntax
checks for all 34 shipped scripts. The base notice inventory matches 87 external
dependency versions and the SQLite inventory matches 94, including build
dependencies. Rust library notices are present and all inventoried file hashes
match. This is an inventory review, not maintainer attribution signoff.

All checked example/playground links resolve inside the packages. Six links in
the performance document point to benchmark instructions/raw evidence that
require a source checkout; this limitation is now explicit in the source docs.
Local `codesign` inspection identifies ad hoc linker signatures with no team
identity. These are not publisher signatures or notarization.

The generated `archive-review.json` alongside `validation.json` records these
checks against the existing archives. Follow-up documentation edits are not
contained in those immutable reviewed archives; final release inputs must be
repackaged after selecting the reviewed source revision.

## Remaining release blockers

- Native baseline validation for all pending targets, including macOS 15 ARM64.
- Maintainer review of changes and dependency/license attribution notices.
- A clean, reviewed source revision and matching `v0.1.0` tag.
- Selection of supported assets and attachment of the corresponding native
  records before publication. Candidates have no publisher signatures or
  notarization; the local macOS binaries have ad hoc linker signatures.

The [release checklist](release-checklist.md) and [compatibility matrix](compatibility.md)
remain the sources for release gates and platform claims. The local run did not
create a commit, tag, published asset, installation into a user's PATH, or release.

## Native GitHub Actions validation

The [validation run](https://github.com/trafficinc/scripta/actions/runs/43551112355)
tests tag `v0.1.0-validation.20261113.6`, source revision
`142df7808fc0ee2d07a99a094fd8afd15e966f3e`, with pinned Rust 1.99.1.
All three jobs passed: Ubuntu 22.04 x86-64/ARM64 and Windows Server 2022 x86-64.

| Native host | Debug default | Debug all features | Dist default | Dist all features |
| --- | ---: | ---: | ---: | ---: |
| [Ubuntu 22.04 x86-64](https://github.com/trafficinc/scripta/actions/runs/43551112355/job/132313528683) | 433 | 434 | 433 | 434 |
| [Ubuntu 22.04 ARM64](https://github.com/trafficinc/scripta/actions/runs/43551112355/job/132313529012) | 433 | 434 | 433 | 434 |
| [Windows Server 2022 x86-64](https://github.com/trafficinc/scripta/actions/runs/43551112355/job/132313529026) | 426 | 427 | 426 | 427 |

All listed suites passed with zero failed or ignored tests. Formatting,
validation-driver unit tests, and workspace/all-target/all-feature Clippy with
warnings denied also passed. Each successful job built both base and SQLite
packages, verified archive checksums and contents, inspected native library
dependencies, ran offline extracted-package smoke tests, and confirmed identical
hashes when repacking identical inputs. Linux dependencies were confined to the
documented system libraries. Windows imports contained system DLLs, without
dynamic MSVC/UCRT, OpenSSL, or SQLite runtime dependencies. Platform-specific
test guards account for the different suite counts; no failing tests were ignored.

Earlier validation attempts exposed Windows local Git path normalization,
transient atomic replacement denials, short-path reporting in a process fixture,
and nonexistent-path acceptance in disk-usage queries. These were fixed with
regression coverage. A concurrent Linux descriptor fixture was isolated from a
child-spawn fixture to prevent inherited locks from interfering with close checks.
The atomic-reader and descriptor-lifecycle assertions remain enforced.

The run artifacts contain full logs, packages, dependency reports, and
`validation.json`, with seven-day retention. Public check annotations retain
compact test counts, package/binary hashes, dependency lists, and smoke/repack
results. The committed [native receipts](validation/native-20261113.6.json)
preserve those summaries, archive/binary SHA-256 hashes, dependency lists,
job URLs, and artifact IDs/digests. These receipts are compact public evidence;
full logs and package smoke records remain in the run artifacts.

This validation-only tag does not publish a release. Documentation
recording these results was added after the tested tag and is not contained in
those immutable candidate archives; final release inputs still need a reviewed
source tag and fresh packages.

## Post-migration native validation

On 2026-11-14, [run 43608759544](https://github.com/trafficinc/scripta/actions/runs/43608759544)
passed on all three native targets at tag `v0.1.0-validation.20261114.2`, source
revision `dc81e10f05bb7c60b7e0b3517749392ce904195d`, with Rust 1.99.1.
This validates the URL/form helpers, primary `scripta` and compatibility `scr`
commands, `.scripta`/`.scta`/`.scr` filenames, and canonical `<?scripta` with
legacy `<?scr` support.

| Native host | Debug default | Debug all features | Dist default | Dist all features |
| --- | ---: | ---: | ---: | ---: |
| Ubuntu 22.04 x86-64 | 447 | 448 | 447 | 448 |
| Ubuntu 22.04 ARM64 | 447 | 448 | 447 | 448 |
| Windows Server 2022 x86-64 | 440 | 441 | 440 | 441 |

All suites passed with zero failed or ignored tests. Formatting, all five
validation-driver tests, and workspace/all-target/all-feature Clippy passed.
Base and SQLite archives passed verification, dependency inspection, offline
extracted-package smoke, checksum verification, and identical-input repacking.
The archives include matching primary and compatibility executables.

The first post-migration Windows run exposed stale `.scr` Git attributes, which
allowed renamed source files to check out as CRLF. The fix enforces LF for all
three source extensions while preserving exact snapshot/benchmark fixture bytes.
An isolated `core.autocrlf=true` checkout regression test reproduced the failure
before the fix and passed afterward; the original formatter assertion remains.

The [native receipts](validation/native-20261114.2.json) retain job/download URLs,
test counts, archive/binary SHA-256 hashes, dependencies, artifact IDs/digests,
and expiration times. Full packages and logs are attached to the run with
seven-day retention. The Windows artifact contains `base/` and `sqlite/` ZIPs;
each contains `scripta.exe` and legacy `scr.exe`.

This is validation evidence, not a published release. These documentation updates
follow the tested tag and do not alter the immutable candidate archives. macOS
15 baseline validation and maintainer publication review remain outstanding.
