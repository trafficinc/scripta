# Installation and uninstallation

## Choose a verified release candidate

Use an archive whose target is marked verified in the release's compatibility
matrix and whose associated native test records show passing results. See
[compatibility.md](compatibility.md) for the current evidence; configured targets
that are still pending are not supported-download promises. This work prepares
release candidates and does not itself publish a GitHub Release.

Download the matching archive and its `.sha256` sidecar from the same trusted
release. `SHA256SUMS` also lists all published candidate archives. A checksum
checks file integrity; it does not replace trust in the release source/signing.
There is no curl-to-shell installer or automatic privileged installation.

The extracted directory contains the standalone executable, LICENSE, dependency
notices, examples, runnable playgrounds and their fixtures, reference
documentation, and release.json with file hashes.
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
`scr-playground` (`scr-playground.exe` on Windows) beside `scr` or on PATH, then
run `scr playground`. This uses your installed interpreter. You may instead set
`SCR_PLAYGROUND_BIN` to the companion executable's full path. The command never
downloads or installs the app automatically. Use `scr playground --no-open` to
print the local URL without opening a browser. Delete the companion and remove
any PATH/environment entry to uninstall it.

## Linux x86-64 / ARM64

After the corresponding native release is verified, select the appropriate target:

```sh
# x86-64; substitute aarch64-unknown-linux-gnu for a verified ARM64 release.
archive=scripta-0.1.0-x86_64-unknown-linux-gnu.tar.gz
sha256sum -c "$archive.sha256"
tar -xzf "$archive"
mkdir -p "$HOME/.local/bin"
install -m 755 scripta-0.1.0-x86_64-unknown-linux-gnu/scr "$HOME/.local/bin/scr"
export PATH="$HOME/.local/bin:$PATH"
scr version --verbose
scr run scripta-0.1.0-x86_64-unknown-linux-gnu/examples/hello.scr
```

Add the PATH export to your shell startup file if needed. Check the glibc/platform
baseline in the matrix. Do not use the GNU/Linux artifact on musl/Alpine.

## macOS ARM64 / x86-64

```sh
# ARM64; substitute x86_64-apple-darwin only after that target is verified.
archive=scripta-0.1.0-aarch64-apple-darwin.tar.gz
shasum -a 256 -c "$archive.sha256"
tar -xzf "$archive"
mkdir -p "$HOME/.local/bin"
install -m 755 scripta-0.1.0-aarch64-apple-darwin/scr "$HOME/.local/bin/scr"
export PATH="$HOME/.local/bin:$PATH"
scr version --verbose
scr run scripta-0.1.0-aarch64-apple-darwin/examples/hello.scr
```

Candidates are not Developer-ID-signed or notarized. A downloaded executable may
require manual approval through macOS security settings after verification. The
release does not claim Developer ID signing or notarization. Use an archive for
the machine's native architecture; Rosetta compatibility is not established.

## Windows x86-64 (PowerShell)

Run this only for a verified Windows candidate:

```powershell
$archive = 'scripta-0.1.0-x86_64-pc-windows-msvc.zip'
$expected = ((Get-Content "$archive.sha256").Trim() -split '\s+')[0]
$actual = (Get-FileHash -Algorithm SHA256 $archive).Hash.ToLowerInvariant()
if ($actual -ne $expected) { throw 'Archive checksum mismatch' }
Expand-Archive -LiteralPath $archive -DestinationPath .
$install = Join-Path $env:LOCALAPPDATA 'Scripta\bin'
New-Item -ItemType Directory -Force $install | Out-Null
Copy-Item '.\scripta-0.1.0-x86_64-pc-windows-msvc\scr.exe' $install
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($userPath -split ';') -notcontains $install) {
    [Environment]::SetEnvironmentVariable('Path', "$install;$userPath", 'User')
}
$env:Path = "$install;$env:Path"
scr version --verbose
scr run .\scripta-0.1.0-x86_64-pc-windows-msvc\examples\hello.scr
```

No administrator access or separate VC redistributable is expected: the release
uses the static MSVC CRT and audits imported DLLs. Windows candidates are not
Authenticode-signed, so OS reputation prompts may occur. No service or registry
file association is installed.

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
./target/dist/scr version --verbose
```

For macOS distribution builds set `MACOSX_DEPLOYMENT_TARGET=15.0` as in the release
workflow. On Windows use an explicit target so host proc macros do not receive the
static CRT flag:

```powershell
cargo test --workspace --target x86_64-pc-windows-msvc --locked
cargo build -p scr-cli --profile dist --target x86_64-pc-windows-msvc --locked
.\target\x86_64-pc-windows-msvc\dist\scr.exe version --verbose
```

Alternatively, `cargo install --path crates/scr-cli --locked` installs scr into
Cargo's bin directory on Unix. On Windows, add
`--target x86_64-pc-windows-msvc` to that install command. This source-install
route requires build tools; downloaded
binary users do not need them.

## Uninstall

For the per-user Unix installation above:

```sh
rm "$HOME/.local/bin/scr"
```

Remove only PATH entries you added if you no longer use that directory. Other
programs may share `.local/bin`; do not delete the directory or unrelated files.
Downloaded/extracted archives can be removed separately. User scripts, data,
backups, and log files are never automatically removed.

For the Windows per-user installation:

```powershell
$install = Join-Path $env:LOCALAPPDATA 'Scripta\bin'
Remove-Item -LiteralPath (Join-Path $install 'scr.exe')
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$remaining = ($userPath -split ';' | Where-Object { $_ -and $_.TrimEnd('\') -ne $install.TrimEnd('\') }) -join ';'
[Environment]::SetEnvironmentVariable('Path', $remaining, 'User')
```

Open a new terminal to use the updated PATH. For a Cargo installation, use
`cargo uninstall scr-cli` instead. There are no daemons, scheduled tasks,
background services, or global runtime packages to remove.
