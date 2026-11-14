# Local release validation — 2026-11-13

This record covers local release-readiness work for Scripta 0.1.0. It is not a
published release or validation of the other candidate platforms.

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
on macOS 15. Linux x86-64/ARM64, macOS Intel, and Windows x86-64 remain pending
native validation. This local run did not use GitHub Actions. Subsequent
[tag-triggered validation](development.md#tag-triggered-linux-and-windows-validation)
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
remain the sources for release gates and platform claims. No commit, tag,
published asset, installation into a user's PATH, or release was performed.
