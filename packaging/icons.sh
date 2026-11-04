#!/usr/bin/env bash
# Regenerate committed icons on macOS; normal builds consume the outputs.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIR="$ROOT/assets/app-icon"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
DEVELOPER="$(xcode-select -p)"
ICTOOL="$DEVELOPER/../Applications/Icon Composer.app/Contents/Executables/ictool"
for tool in sips iconutil python3; do
  command -v "$tool" >/dev/null || { echo "Missing tool: $tool" >&2; exit 1; }
done
[[ -x "$ICTOOL" ]] || { echo "Install Xcode with Icon Composer first" >&2; exit 1; }
python3 -c 'from PIL import Image' || { echo "Install Pillow for ICO packing" >&2; exit 1; }
# sips uses the system SVG renderer. Preserve native Liquid Glass geometry and
# Apple's 824/1024 optical grid for runtime PNGs and legacy ICNS.
sips -s format png "$DIR/vibecut.svg" --out "$DIR/vibecut-1024.png" >/dev/null
"$ICTOOL" "$DIR/AppIcon.icon" --export-image --output-file "$TMP/macos.png" \
  --platform macOS --rendition Default --width 1024 --height 1024 --scale 1
python3 - "$TMP/macos.png" "$DIR/vibecut-macos-1024.png" <<'PYICON'
import sys
from PIL import Image
image = Image.open(sys.argv[1]).convert('RGBA').resize((824, 824), Image.Resampling.LANCZOS)
canvas = Image.new('RGBA', (1024, 1024))
canvas.alpha_composite(image, (100, 100))
canvas.save(sys.argv[2])
PYICON
sips -z 512 512 "$DIR/vibecut-macos-1024.png" --out "$DIR/vibecut-macos-512.png" >/dev/null
SET="$TMP/vibecut.iconset"
mkdir -p "$SET"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$DIR/vibecut-macos-1024.png" --out "$SET/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" "$DIR/vibecut-macos-1024.png" --out "$SET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns -o "$DIR/vibecut.icns" "$SET"
python3 - "$DIR" <<'PY'
import sys
from pathlib import Path
from PIL import Image
icon_dir = Path(sys.argv[1])
Image.open(icon_dir / 'vibecut-1024.png').save(
    icon_dir / 'vibecut.ico', sizes=[(s, s) for s in (16, 20, 24, 32, 40, 48, 64, 128, 256)])
PY
# Brand exports follow their own vector masters; regenerate the whole set.
sips -s format png "$ROOT/docs/brand/logo.svg" --out "$ROOT/docs/brand/logo.png" >/dev/null
sips -s format png "$ROOT/docs/brand/favicon.svg" --out "$TMP/favicon.png" >/dev/null
sips -z 180 180 "$TMP/favicon.png" --out "$ROOT/docs/brand/apple-touch-icon.png" >/dev/null
python3 - "$ROOT/docs/brand" "$TMP/favicon.png" <<'PY'
import sys
from pathlib import Path
from PIL import Image
Image.open(sys.argv[2]).save(Path(sys.argv[1]) / 'favicon.ico', sizes=[(s, s) for s in (16, 32, 48)])
PY
# Compile native appearances for packaging inspection, outside the source tree.
OUT="$ROOT/target/icons/native"
mkdir -p "$OUT"
xcrun actool "$DIR/AppIcon.icon" --compile "$OUT" --platform macosx \
  --minimum-deployment-target 26.0 --app-icon AppIcon --standalone-icon-behavior all \
  --output-partial-info-plist "$OUT/icon-info.plist"
echo "Icons regenerated; inspect exports before committing. Native catalog: $OUT"
