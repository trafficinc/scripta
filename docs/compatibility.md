# Platform compatibility

Release targets and baseline requirements:

| Platform | Rust target | Native baseline | Archive format |
| --- | --- | --- | --- |
| Linux x86-64 | `x86_64-unknown-linux-gnu` | Ubuntu 22.04 x86-64 | `.tar.gz` |
| Linux ARM64 | `aarch64-unknown-linux-gnu` | Ubuntu 22.04 ARM64 | `.tar.gz` |
| macOS ARM64 | `aarch64-apple-darwin` | macOS 15 Apple Silicon | `.tar.gz` |
| Windows x86-64 | `x86_64-pc-windows-msvc` | Windows Server 2022 x86-64 | `.zip` |

Every archive is named `scripta-<version>-<target>.tar.gz` or `.zip`. Windows contains
`scripta.exe` and compatibility `scr.exe`; other platforms contain `scripta` and
compatibility `scr`. Assets are architecture-specific, not
universal binaries. ARM32/ARMv7, musl/Alpine, Windows ARM64, and other targets are
outside the configured release targets. macOS packages target Apple Silicon
(M-series) Macs. Device-specific platforms, including Raspberry Pi, are not
separate release targets.

## Platform library baselines

Linux candidates are built on Ubuntu 22.04/glibc 2.35 and are intended for a
compatible glibc environment with standard `libgcc_s`/libc/math/loader libraries.
macOS release builds set deployment target 15.0 on Apple Silicon. Windows candidates use the MSVC
target and static CRT, with system Windows DLLs.

Native dependency inspection checks Linux/macOS dynamic libraries and Windows
DLL imports. No OpenSSL, Python, Node.js, PHP, JVM, Docker container, or separately
installed language runtime is required by scripta. These statements apply to the interpreter; a script
explicitly calling Process::run still needs its chosen executable, and HTTP calls
need a connection to the requested endpoint. Local scripts need no network.

Use the compiler pinned in [`rust-toolchain.toml`](../rust-toolchain.toml) for
release builds. [`Cargo.toml`](../Cargo.toml) declares the source minimum Rust
version.
