# Release checklist

Use a separate copy for each candidate. Record evidence links alongside completed
items; this template does not track a particular release's status.

## Source and version

- [ ] Select the reviewed source commit and workspace version.
- [ ] Check the pinned compiler, minimum Rust version, lockfile, and build profiles.
- [ ] Review changes, dependency notices, and license/attribution requirements.
- [ ] Check documentation, examples, compatibility notes, and package inputs
  against the selected source.
- [ ] Choose a tag consistent with the workspace version and publication type.
- [ ] Commit release inputs and preserve unrelated working-tree changes.

## Native validation and packaging

Follow the [release procedure](releases.md) and
[platform baselines](compatibility.md) on each native host.

- [ ] Linux x86-64: debug/dist tests, Clippy, package checks, and dependency report.
- [ ] Linux ARM64: debug/dist tests, Clippy, package checks, and dependency report.
- [ ] macOS ARM64: debug/dist tests, Clippy, package checks, and dependency report.
- [ ] Windows x86-64: debug/dist tests, Clippy, package checks, and static-CRT import report.
- [ ] Review default/all-feature tests, minimal source feature checks, collection
  limits/behavior, formatting, and validation/publication script tests.
- [ ] Record compiler host, native OS/architecture, source revision, and results.
- [ ] Review minimum-compiler checks separately from native platform results.
- [ ] Verify archive inventory and checksums, identical-input repacking, and
  extracted-package smoke with no external language runtime on PATH.
- [ ] Review installation/removal, shipped scripts, and dependency notices.
- [ ] Retain per-target records, logs, binary/archive hashes, and smoke records.

## Publication review

- [ ] Review release notes against the source, artifacts, and known limitations.
- [ ] Describe signing/notarization according to the selected artifacts.
- [ ] Check that all required native jobs and package checks succeeded.
- [ ] Keep exactly six uploaded assets: four SQLite-enabled platform packages,
  `SHA256SUMS`, and `validation-evidence.zip`.
- [ ] Check that `SHA256SUMS` covers the packages and evidence ZIP; the ZIP contains
  individual checksums, native/smoke records, logs, and the release manifest.
- [ ] Use a prerelease for candidate tags and a normal release for the exact
  workspace version tag.
- [ ] Publish through the chosen workflow after reviewing its inputs and evidence.
- [ ] Download published assets, verify checksums, and repeat local-script smoke.
- [ ] Preserve published tags, assets, historical receipts, and hashes.

## Promotion without rebuilding

Follow the [manual promotion instructions](releases.md#promote-validated-packages-without-rebuilding)
when reusing a published candidate's packages.

- [ ] Review the source prerelease, exact validated commit, successful native run,
  and all four platform packages and evidence.
- [ ] Ensure the manual promotion workflow is installed on the default branch.
- [ ] Run read-only preparation and compare all four output archive hashes with
  the candidate; review stable metadata and source-run provenance.
- [ ] Dispatch **Promote validated release** on the default branch with the
  reviewed source tag, native run ID, and stable tag. Let the workflow create
  the stable tag with its built-in token.
- [ ] Confirm the stable tag resolves to the validated commit and the published
  release has exactly six assets, with unchanged platform archive hashes.

Local checklist and packaging commands do not publish anything. A normal tag
push starts the [native build and release workflow](development.md#tag-triggered-builds-and-releases).
Manual promotion reuses existing packages and creates its tag without starting
another native build. Neither path moves existing tags or replaces published assets.
