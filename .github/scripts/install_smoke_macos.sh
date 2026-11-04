#!/bin/bash
# Called only on the separate clean release VM, with no media provisioning.
set -euo pipefail
images=()
while IFS= read -r -d '' image; do images+=("$image"); done < <(find target/download -name '*.dmg' -print0)
[ "${#images[@]}" -eq 1 ] || { echo 'Expected exactly one signed DMG' >&2; exit 1; }
mount="$RUNNER_TEMP/vibecut-volume"
mkdir "$mount"
trap 'hdiutil detach "$mount" >/dev/null 2>&1 || true' EXIT
codesign --verify --strict "${images[0]}"
xcrun stapler validate "${images[0]}"
spctl --assess --type open --context context:primary-signature "${images[0]}"
hdiutil attach -nobrowse -readonly -mountpoint "$mount" "${images[0]}"
sudo ditto "$mount/VibeCut.app" /Applications/VibeCut.app
sh "$mount/install-cli.sh"
export PATH="$HOME/.local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
test "$(command -v vibecut-cli)" = "$HOME/.local/bin/vibecut-cli"
vibecut-cli probe target/download/release-smoke.mp4 --json > target/installed-cli-probe.json
python3 packaging/smoke.py --installed /Applications/VibeCut.app --signed \
  --media target/download/release-smoke.mp4 --output target/clean-host-smoke
