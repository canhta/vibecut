#!/usr/bin/env bash
# Install the extracted Linux tarball for the current user (no sudo needed).
set -euo pipefail

usage() {
  echo 'Usage: ./install.sh [--prefix /absolute/path]'
  echo 'Default prefix: ~/.local. For all users: sudo ./install.sh --prefix /usr/local'
}
fail() { echo "error: $*" >&2; exit 1; }

prefix="${HOME:?HOME is not set}/.local"
while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --prefix) [[ "$#" -ge 2 ]] || fail '--prefix needs a path'; prefix="$2"; shift 2 ;;
    -h | --help) usage; exit 0 ;;
    *) usage >&2; fail "unknown argument: $1" ;;
  esac
done
case "$prefix" in /*) ;; *) fail '--prefix must be absolute' ;; esac

source_dir="$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P)"
for binary in vibecut vibecut-cli; do
  [[ -f "$source_dir/bin/$binary" ]] || fail "missing bin/$binary; extract the complete tarball first"
done

app="$prefix/opt/vibecut"
mkdir -p -- "$app" "$prefix/bin" "$prefix/share/applications" "$prefix/share/icons/hicolor/512x512/apps"
rm -rf -- "$app"
cp -R -- "$source_dir/bin" "$source_dir/lib" "$source_dir/compliance" "$app/"
install -m644 -- "$source_dir/vibecut.png" "$prefix/share/icons/hicolor/512x512/apps/vibecut.png"
install -m644 -- "$source_dir/vibecut.desktop" "$prefix/share/applications/vibecut.desktop"
for binary in vibecut vibecut-cli; do
  ln -sfn -- "../opt/vibecut/bin/$binary" "$prefix/bin/$binary"
done
command -v update-desktop-database >/dev/null && update-desktop-database -q "$prefix/share/applications" || true

echo "installed vibecut and vibecut-cli under $app"
echo "launchers: $prefix/bin/vibecut"
