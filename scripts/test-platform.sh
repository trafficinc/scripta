#!/usr/bin/env bash
# Shared Bash 3.2+ launcher. No Python or Rust is needed for an existing binary.
set -euo pipefail
expected_os=${1:?use test-platform-linux.sh or test-platform-macos.sh}
shift
script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
repository=$(CDPATH= cd -- "$script_directory/.." && pwd -P)
invocation_directory=$(pwd -P)
requested_binary=
requested_output=
build=false
while (($#)); do
    case "$1" in
        --scripta|--output)
            option=$1
            if (($# < 2)) || [[ -z $2 ]]; then
                echo "$option requires a value" >&2
                exit 2
            fi
            if [[ $option == --scripta ]]; then requested_binary=$2; else requested_output=$2; fi
            shift 2
            ;;
        --build) build=true; shift ;;
        --help|-h)
            echo 'Usage: bash scripts/test-platform-OS.sh [--scripta PATH | --build] [--output NEW_DIRECTORY]'
            echo 'Runs the shared native smoke suite, including SQLite. Results default to target/platform-tests/.'
            exit 0
            ;;
        *) echo "Unknown option: $1" >&2; exit 2 ;;
    esac
done
if [[ $build == true && -n $requested_binary ]]; then
    echo '--build and --scripta are mutually exclusive' >&2
    exit 2
fi
case "$(uname -s)" in
    Linux) actual_os=linux ;;
    Darwin) actual_os=macos ;;
    *) echo 'Use the Windows PowerShell launcher on Windows.' >&2; exit 2 ;;
esac
if [[ $actual_os != "$expected_os" ]]; then
    echo "This launcher requires $expected_os; this host is $actual_os." >&2
    exit 2
fi
case "$(uname -m)" in
    x86_64) architecture=x86_64 ;;
    arm64|aarch64) architecture=aarch64 ;;
    *) echo 'Supported native hosts are x86-64/ARM64 Linux and Apple Silicon macOS.' >&2; exit 2 ;;
esac
if [[ $actual_os == macos ]]; then
    if [[ $architecture != aarch64 ]]; then
        echo 'Use a native Apple Silicon shell; Intel macOS is not supported.' >&2
        exit 2
    fi
    target=aarch64-apple-darwin
else
    target=$architecture-unknown-linux-gnu
fi
if [[ -n $requested_output ]]; then
    if [[ $requested_output != /* ]]; then requested_output=$invocation_directory/$requested_output; fi
    mkdir -p -- "$(dirname -- "$requested_output")"
    # Exclusive directory creation preserves previous reports.
    mkdir -- "$requested_output"
    results=$(CDPATH= cd -- "$requested_output" && pwd -P)
else
    mkdir -p -- "$repository/target/platform-tests"
    results=$(mktemp -d "$repository/target/platform-tests/$actual_os.XXXXXX")
fi
echo "Results: $results"
uname -a > "$results/host.txt"
if [[ $actual_os == macos ]]; then sw_vers >> "$results/host.txt"; fi
if [[ $build == true ]]; then
    (
        cd -- "$repository"
        rustc -vV > "$results/compiler.txt"
        compiler=$(cat "$results/compiler.txt")
        if [[ $compiler != *"host: $target"$'\n'* ]]; then
            echo "Compiler host must match native target $target." >&2
            exit 2
        fi
        if [[ $actual_os == macos ]]; then export MACOSX_DEPLOYMENT_TARGET=15.0; fi
        cargo build -p scr-cli --release --locked --target "$target"
    ) > "$results/build.log" 2>&1 || { status=$?; cat "$results/build.log" >&2; exit "$status"; }
    requested_binary=$repository/target/$target/release/scripta
fi
if [[ -z $requested_binary ]]; then
    for candidate in "$repository/target/$target/release/scripta" "$repository/target/$target/dist/scripta" \
                     "$repository/target/$target/debug/scripta" "$repository/target/release/scripta" "$repository/target/debug/scripta"; do
        if [[ -f $candidate && -x $candidate ]]; then requested_binary=$candidate; break; fi
    done
    if [[ -z $requested_binary ]]; then requested_binary=$(command -v scripta || true); fi
fi
if [[ -n $requested_binary && $requested_binary != */* ]]; then requested_binary=$(command -v "$requested_binary" || true); fi
if [[ -z $requested_binary || ! -f $requested_binary || ! -x $requested_binary ]]; then
    echo 'No Scripta executable found. Pass --scripta PATH or --build.' >&2
    exit 2
fi
binary_directory=$(CDPATH= cd -- "$(dirname -- "$requested_binary")" && pwd -P)
binary=$binary_directory/$(basename -- "$requested_binary")
echo "Executable: $binary"
printf '%s\n' "$binary" > "$results/executable.txt"
"$binary" version --verbose > "$results/version.txt" 2> "$results/version-stderr.txt" || {
    status=$?; cat "$results/version-stderr.txt" >&2; exit "$status";
}
cat "$results/version.txt"
if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$binary" > "$results/binary.sha256"
elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$binary" > "$results/binary.sha256"
fi
export SCRIPTA_PLATFORM_BINARY=$binary
export SCRIPTA_PLATFORM_OS=$actual_os
export SCRIPTA_PLATFORM_ARCH=$architecture
export SCRIPTA_PLATFORM_FIXTURES=$repository/tests/platform/fixtures
cd -- "$repository"
status=0
"$binary" test tests/platform --format json --show-output > "$results/report.json" 2> "$results/stderr.txt" || status=$?
summary_status=0
"$binary" run tests/platform/fixtures/report.scripta "$results/report.json" || summary_status=$?
if [[ -s $results/stderr.txt ]]; then cat "$results/stderr.txt" >&2; fi
echo "Results: $results"
if ((status != 0)); then exit "$status"; fi
exit "$summary_status"
