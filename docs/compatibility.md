# Compatibility matrix — 0.1.0 release candidates

Configured build targets are not support claims. A platform is verified only when
its executable passes native tests and the extracted-package smoke test.
The [release-validation record](release-validation.md) describes the tested
sources, native environments, and retained local and Actions evidence.

| Platform | Rust target / asset suffix | Evidence available here | Status |
| --- | --- | --- | --- |
| Linux x86-64 | `x86_64-unknown-linux-gnu.tar.gz` | [Native debug/dist tests, dependency inspection and base/SQLite package checks](release-validation.md#native-github-actions-validation) | Verified on Ubuntu 22.04 |
| Linux ARM64 | `aarch64-unknown-linux-gnu.tar.gz` | [Native debug/dist tests, dependency inspection and base/SQLite package checks](release-validation.md#native-github-actions-validation) | Verified on Ubuntu 22.04 |
| macOS ARM64 | `aarch64-apple-darwin.tar.gz` | Local macOS 27.0.1 arm64 debug/dist suites, dependency inspection and extracted-package smoke passed | Verified locally on macOS 27.0.1; macOS 15 validation pending |
| Windows x86-64 | `x86_64-pc-windows-msvc.zip` | [Native debug/dist tests, static-CRT inspection and base/SQLite package checks](release-validation.md#native-github-actions-validation) | Verified on Windows Server 2022 |

The native receipts above describe the historical `scr`/`.scr` revision. The
`scripta`/`.scripta` migration preserves source syntax but requires fresh native
executable and archive validation before publishing the renamed artifacts.

Every archive is named `scripta-0.1.0-TARGET.tar.gz` or `.zip`. Windows contains
`scripta.exe` and compatibility `scr.exe`; other platforms contain `scripta` and
compatibility `scr`. Assets are architecture-specific, not
universal binaries. ARM32/ARMv7, musl/Alpine, Windows ARM64, and other targets are
not part of this release matrix. macOS support is limited to Apple Silicon
(M-series) Macs; Intel Macs are not supported. Raspberry Pi hardware has not
been tested.

## Platform library baselines

Linux candidates are built on Ubuntu 22.04/glibc 2.35 and are intended for a
compatible glibc environment with standard `libgcc_s`/libc/math/loader libraries.
Older glibc distributions are not verified. macOS release builds set deployment
target 15.0 on Apple Silicon; older macOS versions are not verified. Windows
candidates use the MSVC target and static CRT, with system Windows DLLs; the native
validation image is Windows Server 2022. Other Windows versions require their own
validation before a specific-version support claim.

Native dependency inspection checks Linux/macOS dynamic libraries and Windows
DLL imports. No OpenSSL, Python, Node.js, PHP, JVM, Docker container, or separately
installed language
runtime is required by scripta. These statements apply to the interpreter; a script
explicitly calling Process::run still needs its chosen executable, and HTTP calls
need a connection to the requested endpoint. Local scripts need no network.

Rust 1.99.1 is pinned for release builds. The manifests declare Rust 1.85 as the
source minimum. The Rust 1.85.0 workspace/all-target check passed locally on macOS
ARM64; compiler compatibility is distinct from native executable/platform
verification.

## Updating a row

Record the commit/tag, native machine OS/architecture, complete debug and dist test
results, the archive and binary hashes, dependency inspection output, and the
`*.validation.json` generated from the extracted archive. Include the validation
records in release notes. A cross-compilation success, a checksum, or a smoke test
alone does not establish complete native support. Support for an
older OS version is not inferred from success on a newer one.
