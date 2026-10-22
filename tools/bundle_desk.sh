#!/usr/bin/env bash
# Builds the desktop app's release bundles from the release workflow's native
# and desk archives. Each bundle carries `bunting` beside the app, so the app
# opened with no flags shows the connect screen, whose "Practice locally"
# starts or joins the local practice market with it.
#
#   tools/bundle_desk.sh <tag> <archives-dir> <out-dir>
#
# Writes to <out-dir>:
#   Bunting-<tag>-macOS.dmg                       universal Bunting.app (needs macOS: lipo, sips, iconutil, codesign, hdiutil)
#   bunting-desk-<tag>-<linux-target>.tar.gz      bin/bunting-desk, bin/bunting, launcher entry and icon
#   bunting-desk-<tag>-x86_64-pc-windows-msvc.zip bunting-desk.exe and bunting.exe side by side
set -euo pipefail

tag=$1
archives=$(cd "$2" && pwd)
mkdir -p "$3"
out=$(cd "$3" && pwd)
root=$(cd "$(dirname "$0")/.." && pwd)
packaging="$root/apps/bunting-desk/packaging"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Unpacks one release archive (found anywhere under <archives-dir>) into
# $work/<name> and prints that directory.
unpack() {
  local name=$1 archive
  archive=$(find "$archives" -name "$name.tar.gz" -print -quit)
  if [ -z "$archive" ]; then
    echo "missing archive $name.tar.gz under $archives" >&2
    exit 1
  fi
  tar -xzf "$archive" -C "$work"
  echo "$work/$name"
}

for target in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
  native=$(unpack "bunting-$tag-$target")
  desk=$(unpack "bunting-desk-$tag-$target")
  cp "$native/bin/bunting" "$desk/bin/bunting"
  mkdir -p "$desk/share/applications" "$desk/share/icons/hicolor/scalable/apps"
  cp "$packaging/linux/bunting-desk.desktop" "$desk/share/applications/"
  cp "$packaging/bunting.svg" "$desk/share/icons/hicolor/scalable/apps/bunting-desk.svg"
  tar -C "$work" -czf "$out/bunting-desk-$tag-$target.tar.gz" "bunting-desk-$tag-$target"
done

target=x86_64-pc-windows-msvc
native=$(unpack "bunting-$tag-$target")
desk=$(unpack "bunting-desk-$tag-$target")
windows="$work/Bunting-$tag-windows"
mkdir -p "$windows"
cp "$desk/bin/bunting-desk.exe" "$native/bin/bunting.exe" "$desk/README.md" "$desk/RULES.md" "$windows/"
(cd "$work" && zip -qr "$out/bunting-desk-$tag-$target.zip" "Bunting-$tag-windows")

# macOS: one universal app for Apple Silicon and Intel.
version=${tag#v}
short_version=${version%%-*}
app="$work/dmg/Bunting.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
for program in bunting-desk bunting; do
  parts=()
  for target in aarch64-apple-darwin x86_64-apple-darwin; do
    if [ "$program" = bunting ]; then
      dir=$(unpack "bunting-$tag-$target")
    else
      dir=$(unpack "bunting-desk-$tag-$target")
    fi
    parts+=("$dir/bin/$program")
  done
  lipo -create "${parts[@]}" -output "$app/Contents/MacOS/$program"
done
sed -e "s/@VERSION@/$version/g" -e "s/@SHORT_VERSION@/$short_version/g" \
  "$packaging/macos/Info.plist" >"$app/Contents/Info.plist"
iconset="$work/Bunting.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$packaging/bunting.png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" "$packaging/bunting.png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/Bunting.icns"
# Ad hoc signatures: Apple Silicon runs nothing unsigned. Sign the helper
# first, then the bundle that seals it.
codesign --force --sign - "$app/Contents/MacOS/bunting"
codesign --force --sign - "$app"
codesign --verify --deep --strict "$app"
ln -s /Applications "$work/dmg/Applications"
hdiutil create -quiet -volname Bunting -srcfolder "$work/dmg" -ov -format UDZO \
  "$out/Bunting-$tag-macOS.dmg"

ls -l "$out"
