#!/bin/sh
# After dragging VibeCut.app into Applications, install its CLI for this user.
set -eu
app=${1:-/Applications/VibeCut.app}
[ -x "$app/Contents/MacOS/vibecut-cli" ] || { echo 'Install VibeCut.app first' >&2; exit 1; }
app=$(CDPATH= cd -- "$app" && pwd -P)
bin=${2:-"$HOME/.local/bin"}
mkdir -p "$bin"
[ ! -e "$bin/vibecut-cli" ] && [ ! -L "$bin/vibecut-cli" ] || { echo 'CLI destination already exists; inspect it first' >&2; exit 1; }
ln -s "$app/Contents/MacOS/vibecut-cli" "$bin/vibecut-cli"
printf 'CLI installed in %s; add this directory to PATH if needed.\n' "$bin"
