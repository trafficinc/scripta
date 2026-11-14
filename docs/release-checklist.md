# Release checklist — 0.1.0

Check only completed items with evidence. Unchecked platform entries block an
all-platform release.

## Source and version

- [x] Standalone scr binary target; workspace version 0.1.0.
- [x] Rust compiler pinned, Cargo.lock retained, unwind/overflow-safe profiles.
- [x] Required examples and standard-library reference included in package inputs.
- [x] Inspect release-tool diff, unchanged dependency inputs, packaged notice
  inventory, and documented release limitations against local artifacts.
- [ ] Maintainer review of changes and license/attribution notices for publication.
- [x] Reconcile feature-absence and capture-lifetime claims across release notes,
  architecture, and library docs with the current [package manager](packages.md),
  [collector documentation](performance.md), implementation, and tests.
- [ ] Verify the source tag exactly matches v0.1.0 and the intended reviewed commit.
- [ ] Ensure the source tree is clean and source/release settings are committed.

## Local evidence

The current [local validation record](release-validation.md) identifies the
source base and uncommitted changes, native environment, feature variants, and
generated evidence. Earlier artifacts in `dist/` are historical and do not
replace a fresh run against the intended release source.

- [x] Complete macOS ARM64 debug test suite and Clippy passed during preparation.
- [x] Declared MSRV check result recorded.
- [x] Complete native dist-profile test suite passed.
- [x] Native distribution binary dependency inspection passed.
- [x] Archive inventory/checksums verified; identical-input repack has identical hash.
- [x] Extracted archive passed offline smoke with no external language runtime PATH.
- [x] Archive name/hash and local machine/OS recorded in dist validation evidence.
- [x] Local base/SQLite archives pass isolated installation/removal, syntax checks
  of all 34 shipped scripts, and dependency-notice inventory review.

## Required native evidence before advertising all targets

- [x] Linux x86-64 debug/dist/CLI/package tests and dependency report.
- [x] Linux ARM64 debug/dist/CLI/package tests and dependency report.
- [ ] macOS ARM64 debug/dist/CLI/package tests on the declared macOS baseline.
- [ ] macOS x86-64 debug/dist/CLI/package tests and dependency report.
- [x] Windows x86-64 debug/dist/CLI/package tests and static-CRT import report.
- [ ] Review the MSRV check and every relevant test; no ignored/skipped platform failures.
- [ ] Attach per-target validation records; update compatibility rows.

## Candidate review and publication (maintainer action)

- [ ] Verify aggregate SHA256SUMS and all five uniquely named archives.
- [ ] Inspect documentation/examples and installation/uninstallation for every
  target selected for publication; local macOS artifact review is recorded above.
- [x] Describe observed local ad hoc signatures and absent publisher
  signing/notarization without claiming other-platform verification.
- [x] Review local preview limitations and explicit optional network/process behavior.
- [x] Prepare [draft candidate notes](releases.md#draft-candidate-notes) with local
  platform evidence and known limitations; maintainer approval remains pending.
- [ ] Publish chosen validated assets, checksums, validation and dependency reports.
- [ ] Download a published artifact, verify checksum, and repeat local-script smoke.

No tag, release, registry entry, account setting, or deployment is changed merely
by checking this file or running the local packaging commands.
