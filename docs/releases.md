# Release procedure

The release process has one local script and two manual GitHub Actions workflows.
Use the pinned compiler, committed `Cargo.lock`, and native baselines in
[compatibility](compatibility.md).

## 1. Prepare locally

From the workspace root:

```sh
python3 scripts/prepare_release.py
# With Cargo dependencies cached:
python3 scripts/prepare_release.py --offline
```

The script runs local checks, then offers **patch / minor / major / cancel**.
It checks the proposed tag locally and on `origin`, and checks GitHub releases
(including drafts) before changing version files. Git and an authenticated GitHub
CLI (`gh auth login`) are required; `origin` must point to this GitHub repository.
`--offline` applies to Cargo dependencies; release availability still requires
network access. Failures stop preparation without writing a new version.

Preparation updates the workspace version in `Cargo.toml` and local workspace
entries in `Cargo.lock`, preserving dependency pins and existing line endings.
The script prints the prepared version and the next steps. It does not commit,
push, create tags, start Actions, or publish anything.

Review the changes and release notes, **commit and push main**. Notes should cover
user-visible changes, migration, package contents, and known limitations. Local
macOS checks do not establish Linux or Windows support.

## 2. Build release candidate

Once these workflows are on the default branch, open **Actions → Build release
candidate → Run workflow** and select **main**. There are no version, release tag,
or publication inputs. The build reads the version from the selected commit and
stops early if that version already has a tag or release. If it is already used,
prepare a higher version, commit/push it, and build main again.

The workflow validates natively on Linux x86-64/ARM64, Windows x86-64, and macOS 15
Apple Silicon, then packages SQLite-enabled downloads. All four jobs must pass
before assembly. Failed jobs keep any diagnostic artifacts they produced.
The build never creates tags or GitHub Releases, and there are no automatic
branch, pull-request, or tag-push build/publication triggers.

The successful run summary shows the **version, source commit, derived release
tag, build run ID, download link, and publication instructions**. Copy its run ID.
The `scripta-build-<run-id>-<attempt>` artifact contains exactly four platform
archives, `SHA256SUMS`, and `validation-evidence.zip`. It is retained for 30 days;
individual native diagnostic artifacts are retained for seven days.

## 3. Publish release candidate

Open **Actions → Publish release candidate → Run workflow** on the **default
branch**. Enter the successful build's **build run ID**, add optional notes, and
run. A run ID is also the number at the end of the build's Actions run URL.

There is no tag field and no automatic latest-build selection. The workflow
reads the version from the **build's source commit** and derives `v<version>`.
For example, a `0.1.2` build publishes `v0.1.2`, even if main has since advanced.
It verifies the selected run, all four platform jobs, artifact assembly,
artifact identity/availability, checksums, and native evidence. Existing releases
or unrelated tags are reported before downloading, with instructions to prepare
and build a newer version. An expired or deleted artifact requires a new build.

Publication tags the **built commit**, not the current workflow commit. The four
platform archives are copied byte-for-byte without rebuilding. Release metadata,
`validation-evidence.zip`, and `SHA256SUMS` are regenerated to record the source
run, attempt, artifact, and original evidence checksum. Native validation records,
logs, smoke receipts, and package hashes remain unchanged. Exactly six assets
are uploaded to a draft first; the release becomes public only when every upload
succeeds. These version releases are normal stable releases.

If upload fails, rerun **Publish release candidate** with the **same build run
ID**. It can resume its own tag/draft. Published releases cannot be replaced and
tags are never moved. A concurrent build retry or artifact replacement stops
publication. The workflow uses the built-in `GITHUB_TOKEN`; it does not compile
Rust or start another build.

The previous promotion workflow and tag-and-build helper have been retired.
Use these two manual workflows for future releases; existing published releases,
validation receipts, and tags are preserved.

Python 3.9 or newer, Git, Rustup, and a C compiler for bundled SQLite are required
for local preparation. Publishing runs on GitHub and needs no personal access token.

## Pinned inputs and profiles

The `release` profile uses opt-level 3, thin LTO, one codegen unit, checked overflow,
and panic unwinding. `dist` inherits release and strips symbols. Keep unwinding
for resource cleanup. Windows uses `+crt-static`; retain it when setting
`RUSTFLAGS` and pass an explicit Windows target to Cargo.

Set `SCR_BUILD_COMMIT` explicitly to embed a source revision; otherwise the build
reports `unknown`. `scripta version --verbose` reports the compiler, target,
profile, commit metadata, and built-in extensions.

Archives contain `scripta`/`scripta.exe` and an identical compatibility
`scr`/`scr.exe`. File order, permissions, ownership, and archive timestamps are
normalized. `SOURCE_DATE_EPOCH` defaults to zero; use the commit timestamp for a
release. Identical packaging inputs produce identical archives, but compiler
output can vary across SDKs and linkers.

## Local native package procedure

Set `RELEASE_VERSION` to the workspace version. On macOS ARM64:

```sh
export MACOSX_DEPLOYMENT_TARGET=15.0
cargo build -p scr-cli --profile dist --target aarch64-apple-darwin --locked
cargo run -p scr-release --locked --target aarch64-apple-darwin -- package --binary target/aarch64-apple-darwin/dist/scripta --target aarch64-apple-darwin --out-dir dist/local-standard
cargo run -p scr-release --locked --target aarch64-apple-darwin -- verify --archive dist/local-standard/scripta-${RELEASE_VERSION}-aarch64-apple-darwin.tar.gz
cargo run -p scr-release --locked --target aarch64-apple-darwin -- smoke --archive dist/local-standard/scripta-${RELEASE_VERSION}-aarch64-apple-darwin.tar.gz
cargo run -p scr-release --locked --target aarch64-apple-darwin -- checksums --dir dist/local-standard
cargo run -p scr-release --locked --target aarch64-apple-darwin -- verify-checksums --dir dist/local-standard
```

Use a fresh output directory for each packaging run. Packages include reference
documentation, dependency notices, examples, and playgrounds. The maintainer-only
`scr-release` tool is not required by the installed interpreter.

The CLI and packager include SQLite by default. Custom minimal source builds use
`--no-default-features` for the CLI and `--features none` for the packager. They
are not separate official downloads. See [building with SQLite](sqlite.md#building-with-sqlite).

The package manifest inventories every payload file. Verification rejects unsafe
paths, duplicate entries, missing files, and hash mismatches. SHA-256 checksums
provide integrity, not publisher authentication. Review dependency notices and
license attribution before publication.

## Limitations

- macOS packages target Apple Silicon; Intel macOS packages are not provided.
- Packages are not notarized or publisher-signed. No MSI, PKG, automatic updater,
  or privileged installer is provided.
- Platform targets and deployment settings do not establish support on other OS
  versions or architectures.
- Scripta executes trusted scripts with the user's permissions; filesystem root
  policies are not an OS sandbox. Resource budgets are logical limits, not a
  process heap cap.
- Scripts using HTTP or external commands still need their endpoints or programs.

See [installation](install.md), [standard library](standard-library.md), and
[examples](../examples/README.md).
