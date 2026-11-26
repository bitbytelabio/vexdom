#!/bin/sh
# Lays out Vexdom.app: the GPUI desktop app with the `vexdom` binary beside it (desktop/src/bridge.rs
# prefers a sibling `vexdom`), the app icon, and the unpacked Chrome extension under Resources
# (app/src/cli/install.rs `packaged_extension`). Signed ad hoc; there is no Developer ID, so a
# downloaded copy needs `xattr -dr com.apple.quarantine Vexdom.app`.
#
#   scripts/bundle-mac.sh <vexdom-bin> <desktop-bin> <extension-dir> <version> <out-dir>
set -eu

if [ "$#" -ne 5 ]; then
  echo "usage: $0 <vexdom-bin> <desktop-bin> <extension-dir> <version> <out-dir>" >&2
  exit 2
fi
vexdom_bin=$1
desktop_bin=$2
extension_dir=$3
version=$4
out_dir=$5

root=$(cd "$(dirname "$0")/.." && pwd)
# brand/build.py compiles both from brand/AppIcon.icon: Assets.car for macOS 26 and later,
# AppIcon.icns for older ones.
icon="$root/brand/AppIcon.icns"
assets="$root/brand/Assets.car"

for file in "$vexdom_bin" "$desktop_bin" "$icon" "$assets"; do
  [ -f "$file" ] || { echo "no file $file" >&2; exit 1; }
done
[ -f "$extension_dir/manifest.json" ] || { echo "no manifest.json in $extension_dir" >&2; exit 1; }

app="$out_dir/Vexdom.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources/extension"

install -m 755 "$desktop_bin" "$app/Contents/MacOS/vexdom-desktop"
install -m 755 "$vexdom_bin" "$app/Contents/MacOS/vexdom"
cp "$icon" "$app/Contents/Resources/AppIcon.icns"
cp "$assets" "$app/Contents/Resources/Assets.car"
cp -R "$extension_dir" "$app/Contents/Resources/extension/chrome-mv3"

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key>
  <string>vexdom-desktop</string>
  <key>CFBundleIdentifier</key>
  <string>com.vexdom.desktop</string>
  <key>CFBundleName</key>
  <string>Vexdom</string>
  <key>CFBundleDisplayName</key>
  <string>Vexdom</string>
  <key>CFBundleIconFile</key>
  <string>AppIcon</string>
  <key>CFBundleIconName</key>
  <string>AppIcon</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>$version</string>
  <key>CFBundleVersion</key>
  <string>$version</string>
  <key>LSMinimumSystemVersion</key>
  <string>14.0</string>
  <key>LSApplicationCategoryType</key>
  <string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSPrincipalClass</key>
  <string>NSApplication</string>
  <key>NSHumanReadableCopyright</key>
  <string>Apache-2.0</string>
</dict>
</plist>
EOF

plutil -lint "$app/Contents/Info.plist"
codesign --force --deep --sign - "$app"
codesign --verify --deep --strict "$app"
echo "$app"
