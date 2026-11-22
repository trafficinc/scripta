# Installation and uninstallation

## Choose a platform package

Select a release from [GitHub Releases](https://github.com/trafficinc/scripta/releases)
and choose the archive for your OS and architecture. Consult the
[platform baselines](compatibility.md) and the selected release's native records
for its tested environment.

Download the platform archive, `SHA256SUMS`, and `validation-evidence.zip` from
that same release. `SHA256SUMS` covers the four platform packages and the evidence
ZIP. Individual `.sha256` files, native/smoke records, logs, and the release
manifest are bundled inside the ZIP; they are not separate release downloads.
The commands below select the checksum entry for the archive being installed,
so other platform archives need not be downloaded. They require exactly one
matching checksum entry and stop before extraction if verification fails.

A checksum checks file integrity; obtain both the archive and checksums from a
trusted release source. Consult that release's notes for signing or notarization
details. Packages are not publisher-signed or notarized and do not include an
MSI, PKG, automatic updater, or privileged installer. Installation uses per-user
paths and ordinary archive tools.

The extracted directory contains the standalone executable, LICENSE, dependency
notices, examples, runnable playgrounds and their fixtures, reference
documentation, and release.json with file hashes.
Every standard platform package includes bundled SQLite; no extra database
package, server, or installed SQLite library is required. The primary executable is `scripta` (`scripta.exe` on Windows). Archives also
include an identical compatibility executable named `scr` (`scr.exe`). Put the
primary executable on PATH; optionally copy the compatibility executable beside
it if older commands or shebangs need `scr`. Both use the same implementation.
Only the executable needs to be on PATH. Python, Node.js, PHP, Docker, Rust, Cargo,
and a network connection are not needed to run local scripts after installation.
Archive extraction/checksum utilities are installation tools, not interpreter
runtime dependencies.

Developer documentation can link to benchmark harnesses and raw measurements in
the source repository. Those source-checkout files are not included in the
binary archive; runnable examples and playground fixtures are included.

## Optional local browser playground

The playground application is a separate project and download. Its editor and
server are not included in Scripta source, builds, or normal interpreter archives.
The standalone playground download bundles its own interpreter and launcher.

For an existing Scripta installation, put the separately downloaded
`scr-playground` (`scr-playground.exe` on Windows) beside `scripta` or on PATH, then
run `scripta playground`. This uses your installed interpreter. You may instead set
`SCR_PLAYGROUND_BIN` to the companion executable's full path. The command never
downloads or installs the app automatically. Use `scripta playground --no-open` to
print the local URL without opening a browser. Delete the companion and remove
any PATH/environment entry to uninstall it.

## Linux x86-64 / ARM64

Run these commands beside the downloaded archive and `SHA256SUMS`. Replace
`REPLACE_WITH_RELEASE_VERSION` with the version in the archive filename:

```sh
release_version=REPLACE_WITH_RELEASE_VERSION
# For ARM64, use aarch64-unknown-linux-gnu.
package="scripta-${release_version}-x86_64-unknown-linux-gnu"
archive="$package.tar.gz"
checksum_entry=$(awk -v archive="$archive" '
  $2 == archive && NF == 2 && length($1) == 64 && $1 !~ /[^0-9a-f]/ { print; found++ }
  END { if (found != 1) exit 1 }
' SHA256SUMS) || exit 1
printf '%s\n' "$checksum_entry" | sha256sum -c - || exit 1
tar -xzf "$archive"
mkdir -p "$HOME/.local/bin"
install -m 755 "$package/scripta" "$HOME/.local/bin/scripta"
export PATH="$HOME/.local/bin:$PATH"
scripta version --verbose
scripta run "$package/examples/hello.scripta"
```

Add the PATH export to your shell startup file if needed. Check the glibc/platform
baseline in the matrix. Do not use the GNU/Linux artifact on musl/Alpine.

## macOS Apple Silicon (M-series)

Run beside the downloaded archive and `SHA256SUMS`, substituting its version:

```sh
release_version=REPLACE_WITH_RELEASE_VERSION
package="scripta-${release_version}-aarch64-apple-darwin"
archive="$package.tar.gz"
checksum_entry=$(awk -v archive="$archive" '
  $2 == archive && NF == 2 && length($1) == 64 && $1 !~ /[^0-9a-f]/ { print; found++ }
  END { if (found != 1) exit 1 }
' SHA256SUMS) || exit 1
printf '%s\n' "$checksum_entry" | shasum -a 256 -c - || exit 1
tar -xzf "$archive"
mkdir -p "$HOME/.local/bin"
install -m 755 "$package/scripta" "$HOME/.local/bin/scripta"
export PATH="$HOME/.local/bin:$PATH"
scripta version --verbose
scripta run "$package/examples/hello.scripta"
```

A downloaded executable may require approval through macOS security settings.
Review its source and the release's signing/notarization details before approval.
Use the Apple Silicon archive for M-series Macs.

## Windows x86-64 (PowerShell)

Run beside the downloaded ZIP and `SHA256SUMS`, substituting its version:

```powershell
$releaseVersion = 'REPLACE_WITH_RELEASE_VERSION'
$package = "scripta-$releaseVersion-x86_64-pc-windows-msvc"
$archive = "$package.zip"
$pattern = '^[0-9a-f]{64}  ' + [regex]::Escape($archive) + '$'
$entries = @(Get-Content -LiteralPath 'SHA256SUMS' | Where-Object { $_ -cmatch $pattern })
if ($entries.Count -ne 1) { throw 'Expected one archive entry in SHA256SUMS' }
$expected = $entries[0].Substring(0, 64)
$actual = (Get-FileHash -Algorithm SHA256 $archive).Hash.ToLowerInvariant()
if ($actual -ne $expected) { throw 'Archive checksum mismatch' }
Expand-Archive -LiteralPath $archive -DestinationPath .
$install = Join-Path $env:LOCALAPPDATA 'Scripta\bin'
New-Item -ItemType Directory -Force $install | Out-Null
Copy-Item -LiteralPath (Join-Path $package 'scripta.exe') -Destination $install
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($userPath -split ';') -notcontains $install) {
    [Environment]::SetEnvironmentVariable('Path', "$install;$userPath", 'User')
}
$env:Path = "$install;$env:Path"
scripta version --verbose
scripta run (Join-Path $package 'examples\hello.scripta')
```

No administrator access or separate VC redistributable is expected: the release
uses the static MSVC CRT and audits imported DLLs. Review the release's signing
details if Windows displays reputation prompts. Copying the executable does not
install a service or registry file association.

## Build from source

The README includes separate [macOS, Linux, and Windows build instructions](../README.md#build-from-source)
with linker prerequisites, target selection, and commands to run the resulting executable.

A source build requires Rust/Cargo and the native C linker/build tools. Dependencies
are downloaded during the build unless already cached; this is separate from
running an installed interpreter offline. Rustup honors rust-toolchain.toml and
installs the pinned release compiler/components.

```sh
# Native Unix build; use the actual host target from rustc -vV.
cargo test --workspace --locked
cargo build -p scr-cli --profile dist --locked
./target/dist/scripta version --verbose
```

For macOS distribution builds set `MACOSX_DEPLOYMENT_TARGET=15.0` as in the release
workflow. On Windows use an explicit target so host proc macros do not receive the
static CRT flag:

```powershell
cargo test --workspace --target x86_64-pc-windows-msvc --locked
cargo build -p scr-cli --profile dist --target x86_64-pc-windows-msvc --locked
.\target\x86_64-pc-windows-msvc\dist\scripta.exe version --verbose
```

Alternatively, `cargo install --path crates/scr-cli --locked` installs both `scripta` and `scr` into
Cargo's bin directory on Unix. On Windows, add
`--target x86_64-pc-windows-msvc` to that install command. This source-install
route requires build tools; downloaded
binary users do not need them.

## Uninstall

If you installed the optional compatibility command, remove its `scr`/`scr.exe`
file from the same installation directory too. Existing `.scr` source files do
not need renaming. The `.scta` shorthand is also accepted.

For the per-user Unix installation above:

```sh
rm "$HOME/.local/bin/scripta"
```

Remove only PATH entries you added if you no longer use that directory. Other
programs may share `.local/bin`; do not delete the directory or unrelated files.
Downloaded/extracted archives can be removed separately. User scripts, data,
backups, and log files are never automatically removed.

For the Windows per-user installation:

```powershell
$install = Join-Path $env:LOCALAPPDATA 'Scripta\bin'
Remove-Item -LiteralPath (Join-Path $install 'scripta.exe')
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$remaining = ($userPath -split ';' | Where-Object { $_ -and $_.TrimEnd('\') -ne $install.TrimEnd('\') }) -join ';'
[Environment]::SetEnvironmentVariable('Path', $remaining, 'User')
```

Open a new terminal to use the updated PATH. For a Cargo installation, use
`cargo uninstall scr-cli` instead. If you separately configured cron jobs or a
service for automation or `scripta serve`, stop and remove that configuration
through its scheduler or service manager. See
[service management](web-servers.md#deployment-and-service-management).
