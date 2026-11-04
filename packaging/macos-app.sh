#!/bin/sh
# Native package staging; all inputs and release gates are documented beside this file.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
exec python3 "$root/packaging/package.py" --os macos "$@"
