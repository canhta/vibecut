#!/bin/sh
# Regenerate the established exports with the Rust packaging owner.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
exec cargo xtask dist icons "$@"
