#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${APP_DIR}/../.." && pwd)"

VERSION="${BUNTING_TERMINAL_VERSION:-0.1.0}"
BUILD_NUMBER="${BUNTING_TERMINAL_BUILD_NUMBER:-1}"
BINARY="${1:-${APP_DIR}/target/aarch64-apple-darwin/release/bunting-terminal}"
DIST_DIR="${2:-${APP_DIR}/dist}"
APP_NAME="Bunting Market Terminal"
APP_BUNDLE="${DIST_DIR}/${APP_NAME}.app"
DMG_BASENAME="Bunting-Market-Terminal-v${VERSION}-macos-arm64"
DMG_PATH="${DIST_DIR}/${DMG_BASENAME}.dmg"
STAGING_DIR="${DIST_DIR}/dmg-root"
ICONSET_DIR="${DIST_DIR}/BuntingTerminal.iconset"
ICON_PNG="${DIST_DIR}/BuntingTerminal-1024.png"

if [[ ! -f "${BINARY}" ]]; then
  echo "missing ARM64 terminal binary: ${BINARY}" >&2
  exit 1
fi

ARCHS="$(lipo -archs "${BINARY}")"
if [[ " ${ARCHS} " != *" arm64 "* ]]; then
  echo "expected an arm64 Mach-O binary, got: ${ARCHS}" >&2
  exit 1
fi

rm -rf "${DIST_DIR}"
mkdir -p \
  "${APP_BUNDLE}/Contents/MacOS" \
  "${APP_BUNDLE}/Contents/Resources" \
  "${STAGING_DIR}" \
  "${ICONSET_DIR}"

install -m 0755 "${BINARY}" "${APP_BUNDLE}/Contents/MacOS/bunting-terminal"
install -m 0644 "${APP_DIR}/macos/Info.plist" "${APP_BUNDLE}/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString ${VERSION}" "${APP_BUNDLE}/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion ${BUILD_NUMBER}" "${APP_BUNDLE}/Contents/Info.plist"

cat > "${DIST_DIR}/make_icon.swift" <<'SWIFT'
import AppKit

let output = CommandLine.arguments[1]
let size = NSSize(width: 1024, height: 1024)
let image = NSImage(size: size)
image.lockFocus()

guard let context = NSGraphicsContext.current?.cgContext else {
    fatalError("missing CoreGraphics context")
}

let outer = NSBezierPath(roundedRect: NSRect(x: 42, y: 42, width: 940, height: 940), xRadius: 190, yRadius: 190)
let gradient = NSGradient(colors: [
    NSColor(calibratedRed: 0.025, green: 0.035, blue: 0.045, alpha: 1),
    NSColor(calibratedRed: 0.07, green: 0.10, blue: 0.13, alpha: 1)
])!
gradient.draw(in: outer, angle: -90)

NSColor(calibratedWhite: 1, alpha: 0.055).setStroke()
for offset in stride(from: 170, through: 850, by: 136) {
    let vertical = NSBezierPath()
    vertical.move(to: NSPoint(x: offset, y: 150))
    vertical.line(to: NSPoint(x: offset, y: 874))
    vertical.lineWidth = 2
    vertical.stroke()

    let horizontal = NSBezierPath()
    horizontal.move(to: NSPoint(x: 150, y: offset))
    horizontal.line(to: NSPoint(x: 874, y: offset))
    horizontal.lineWidth = 2
    horizontal.stroke()
}

let chart = NSBezierPath()
chart.move(to: NSPoint(x: 146, y: 298))
chart.curve(to: NSPoint(x: 342, y: 468), controlPoint1: NSPoint(x: 225, y: 320), controlPoint2: NSPoint(x: 270, y: 515))
chart.curve(to: NSPoint(x: 514, y: 420), controlPoint1: NSPoint(x: 410, y: 430), controlPoint2: NSPoint(x: 450, y: 375))
chart.curve(to: NSPoint(x: 666, y: 622), controlPoint1: NSPoint(x: 590, y: 468), controlPoint2: NSPoint(x: 590, y: 620))
chart.curve(to: NSPoint(x: 878, y: 744), controlPoint1: NSPoint(x: 744, y: 626), controlPoint2: NSPoint(x: 788, y: 708))
chart.lineWidth = 34
chart.lineCapStyle = .round
chart.lineJoinStyle = .round
NSColor(calibratedRed: 0.18, green: 0.82, blue: 0.55, alpha: 1).setStroke()
chart.stroke()

let dot = NSBezierPath(ovalIn: NSRect(x: 836, y: 702, width: 84, height: 84))
NSColor(calibratedRed: 0.96, green: 0.71, blue: 0.16, alpha: 1).setFill()
dot.fill()

let paragraph = NSMutableParagraphStyle()
paragraph.alignment = .left
let attributes: [NSAttributedString.Key: Any] = [
    .font: NSFont.systemFont(ofSize: 420, weight: .black),
    .foregroundColor: NSColor.white,
    .paragraphStyle: paragraph,
    .kern: -30
]
("B" as NSString).draw(in: NSRect(x: 150, y: 420, width: 520, height: 490), withAttributes: attributes)

context.setShadow(offset: CGSize(width: 0, height: -16), blur: 28, color: NSColor.black.withAlphaComponent(0.35).cgColor)
NSColor(calibratedWhite: 1, alpha: 0.16).setStroke()
outer.lineWidth = 7
outer.stroke()

image.unlockFocus()

guard let tiff = image.tiffRepresentation,
      let bitmap = NSBitmapImageRep(data: tiff),
      let png = bitmap.representation(using: .png, properties: [:]) else {
    fatalError("failed to encode icon PNG")
}
try png.write(to: URL(fileURLWithPath: output))
SWIFT

swift "${DIST_DIR}/make_icon.swift" "${ICON_PNG}"

while read -r name pixels; do
  sips -z "${pixels}" "${pixels}" "${ICON_PNG}" --out "${ICONSET_DIR}/${name}" >/dev/null
 done <<'SIZES'
icon_16x16.png 16
icon_16x16@2x.png 32
icon_32x32.png 32
icon_32x32@2x.png 64
icon_128x128.png 128
icon_128x128@2x.png 256
icon_256x256.png 256
icon_256x256@2x.png 512
icon_512x512.png 512
icon_512x512@2x.png 1024
SIZES

iconutil -c icns "${ICONSET_DIR}" -o "${APP_BUNDLE}/Contents/Resources/BuntingTerminal.icns"

cat > "${APP_BUNDLE}/Contents/Resources/README.txt" <<EOF_README
Bunting Market Terminal ${VERSION}

Architecture: Apple Silicon (arm64)
Connection: configure the existing Bunting terminal profile and credential environment variables before launch.
Signing: ad-hoc signed for preview distribution; this build is not Apple-notarized.
EOF_README

codesign --force --deep --sign - --timestamp=none "${APP_BUNDLE}"
codesign --verify --deep --strict --verbose=2 "${APP_BUNDLE}"

cp -R "${APP_BUNDLE}" "${STAGING_DIR}/"
ln -s /Applications "${STAGING_DIR}/Applications"
cat > "${STAGING_DIR}/Install.txt" <<EOF_INSTALL
Drag “${APP_NAME}” to Applications.

This preview is ad-hoc signed and not notarized. On first launch, macOS may require Control-click → Open.
EOF_INSTALL

hdiutil create \
  -volname "Bunting Market Terminal" \
  -srcfolder "${STAGING_DIR}" \
  -ov \
  -format UDZO \
  "${DMG_PATH}"

shasum -a 256 "${DMG_PATH}" > "${DMG_PATH}.sha256"
file "${APP_BUNDLE}/Contents/MacOS/bunting-terminal"
ls -lh "${DMG_PATH}" "${DMG_PATH}.sha256"

echo "DMG_PATH=${DMG_PATH}"
