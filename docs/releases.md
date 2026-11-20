# Release procedure

Use the compiler pinned in [`rust-toolchain.toml`](../rust-toolchain.toml), the
committed `Cargo.lock`, and a reviewed source commit. Release targets and OS
baselines are listed in [compatibility](compatibility.md).

## Prepare a version

From the workspace root:

```sh
python3 scripts/prepare_release.py
# With all Cargo dependencies cached:
python3 scripts/prepare_release.py --offline
```

The script runs local checks, then prompts for a patch, minor, or major bump.
Cancel is the default. It updates the workspace version and package entries in
`Cargo.lock` without changing dependency pins. Review the changes and release
notes, commit the intended source, and push the branch.

Release notes should describe user-visible changes, migration steps, package
contents, and known limitations. Describe signing or notarization according to
the artifacts actually shipped.

## Automatic publication on tag pushes

Run `python3 scripts/github_release.py` from a clean checkout whose remote branch
matches the local commit. GitHub CLI must be authenticated, and the workflow must
be present on the default branch. The script runs the four-platform checks in
Actions, then prompts for the matching tag before creating and pushing it.
Enter cancels. Use `--remote NAME` for a remote other than `origin`, or
`--run-id RUN_ID` to resume an interrupted run.

The [native build workflow](../.github/workflows/native-validation.yml) runs on
every tag push and on manual dispatch. Branch pushes and pull requests do not
publish releases. After all native jobs succeed, the workflow publishes a GitHub
Release. The exact `v<workspace-version>` tag produces a normal release; other
tags produce prereleases. Version-shaped tags must match the workspace version.

Each release contains exactly six assets: four platform archives with SQLite,
`SHA256SUMS`, and `validation-evidence.zip`. Supporting logs, checksums, native
records, and the release manifest stay inside that ZIP. Uploads remain in a draft
until every asset succeeds. Existing published assets and tags are never replaced.

Python 3.8 or newer, Git, Rustup, and a C compiler for bundled SQLite are required
by the release tools.

## Promote validated packages without rebuilding

On the default branch, open Actions and manually run
[Promote validated release](../.github/workflows/promote-release.yml).

| Input | Meaning |
| --- | --- |
| `validation_tag` | Published candidate tag whose packages will be reused |
| `validation_run_id` | Successful native build and publication run for that candidate |
| `release_tag` | Stable tag matching the workspace version at the candidate commit |

Let the workflow create the stable tag with its built-in `GITHUB_TOKEN`; pushing
that tag yourself starts another native build. Promotion copies the four platform
archives byte-for-byte and regenerates release metadata and checksums. It does
not rebuild packages or move existing tags. Rerun with the same inputs to resume
a failed draft upload.

For a local preview, set `CANDIDATE_TAG`, `NATIVE_RUN_ID`, and `STABLE_TAG`, then run:

```sh
python3 scripts/promote_release.py --repository trafficinc/scripta \
  --validation-tag "$CANDIDATE_TAG" \
  --validation-run-id "$NATIVE_RUN_ID" --release-tag "$STABLE_TAG" \
  --output dist/promotion-preview
```

Omitting `--publish` prepares files locally without creating a tag or release.

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
