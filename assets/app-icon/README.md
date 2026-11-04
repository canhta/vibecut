# App icon

Original artwork by the VibeCut maintainer, MIT (see [LICENSE](../../LICENSE)).

## Palette and geometry

A 1024-unit square with a near-black `#0F1113` (the UI's label-on-lime colour) background and lime `#D1FE17`
foreground. Two curved pieces form a right-facing play mark, scaled 1.2 around
the center. The flat app tile is full bleed; the web favicon has a 224-unit radius.
The macOS source uses extended-sRGB background `(0.05882, 0.06667, 0.07451)`;
its two layers have neutral shadow opacity 0.25 and translucency 0.15. Icon Composer
owns Liquid Glass appearance settings. The script places the native macOS export on Apple's 824/1024 optical grid
with transparent margins/corners for runtime PNGs and legacy ICNS.

## Files

| File | Format/size | Use |
|---|---|---|
| `vibecut.svg` | SVG, 1024-unit square | Canonical flat app tile; light surfaces |
| `vibecut-1024.png` | PNG, 1024 × 1024 | Flat app/render source |
| `AppIcon.icon/` | Layered bundle: icon.json and two SVGs | Editable native macOS icon source |
| `vibecut-macos-1024.png` | PNG, 1024 × 1024 | Icon Composer Default render, transparent corners |
| `vibecut-macos-512.png` | PNG, 512 × 512 | macOS runtime Dock/window icon |
| `vibecut.icns` | ICNS, 16–1024 px | macOS bundle fallback |
| `vibecut.ico` | ICO, 16–256 px | Windows executable and runtime icon |
| `../../docs/brand/logo.svg`, `logo.png` | SVG/PNG, 1024 square | Logo on dark surfaces; README |
| `../../docs/brand/favicon.svg`, `favicon.ico` | SVG/ICO | Web/document favicon |
| `../../docs/brand/apple-touch-icon.png` | PNG, 180 × 180 | Touch bookmark |

## Platform wiring

The desktop app embeds the 512 px macOS PNG and sets
[`ViewportBuilder::with_icon`](https://docs.rs/egui/latest/egui/viewport/struct.ViewportBuilder.html#method.with_icon).
For a `.app`, copy `vibecut.icns` to Resources and set `CFBundleIconFile` accordingly.
For native Liquid Glass appearances, compile `AppIcon.icon` with Xcode/actool,
copy `Assets.car` and merge generated icon plist entries. Apple documents
[adding the bundle to Xcode](https://developer.apple.com/documentation/xcode/creating-your-app-icon-using-icon-composer).
Observed locally: the regeneration command compiles the bundle to `Assets.car`
and ICNS; actual packaged app appearance remains unverified until the app lands.
Windows wiring: the app build.rs embeds `vibecut.ico` as an executable resource;
the runtime icon covers the window/task switcher. Windows appearance is unverified.

## Regenerate

Run `packaging/icons.sh` on macOS with Xcode/Icon Composer, sips, iconutil, Python
and Pillow installed. It renders the flat SVG, exports the layered Default macOS
appearance, creates runtime PNG/ICNS/ICO and refreshes brand vector exports together.
It also writes native catalog/plist inspection output under `target/icons/native/`.
Edit the bundle's layers and the matching flat vectors together; inspect all outputs
and attribution before committing. Committed exports let normal builds avoid these
tools. The desktop app supplies a development bundle script; see [app documentation](../../apps/vibecut/README.md#platforms-and-evidence).
