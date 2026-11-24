#!/usr/bin/env bash
set -euo pipefail
script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
exec bash "$script_directory/test-platform.sh" macos "$@"
