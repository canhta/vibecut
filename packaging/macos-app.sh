#!/bin/sh
# Native package staging; all inputs and release gates are documented beside this file.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
exec cargo xtask dist package --os macos "$@"
