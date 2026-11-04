#!/bin/sh
# Development bundle. Release signing/notarization and FFmpeg receipts belong to packaging verification.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
binary=${1:?usage: packaging/macos-app.sh /absolute/path/to/vibecut /output/VibeCut.app}
bundle=${2:?output bundle required}
[ ! -e "$bundle" ] || { echo 'Refusing to replace an existing bundle' >&2; exit 1; }
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp "$binary" "$bundle/Contents/MacOS/vibecut"
cp "$root/assets/app-icon/vibecut.icns" "$bundle/Contents/Resources/vibecut.icns"
cat > "$bundle/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>vibecut</string>
<key>CFBundleIdentifier</key><string>com.vibecut.editor</string>
<key>CFBundleName</key><string>VibeCut</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>0.1.0</string>
<key>CFBundleVersion</key><string>1</string>
<key>CFBundleIconFile</key><string>vibecut.icns</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
# Native layered appearance is optional for this development bundle; committed ICNS
# remains available on machines without the Xcode icon compiler.
if [ -f "$root/target/icons/native/Assets.car" ]; then
    cp "$root/target/icons/native/Assets.car" "$bundle/Contents/Resources/Assets.car"
fi
plutil -lint "$bundle/Contents/Info.plist"
echo "$bundle"
