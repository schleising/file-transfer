#!/usr/bin/env bash
# Build File Transfer.app locally and install to /Applications (personal use).
# Does not require cargo-bundle — assembles a minimal .app from the release binary.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"

# Local Network privacy (macOS 15+) tracks identity by code signature (TN3179).
# Prefer an Apple-issued identity so the grant survives rebuilds; otherwise ad-hoc.
sign_app() {
  local app="$1"
  local identity=""
  identity="$(security find-identity -v -p codesigning 2>/dev/null | awk '/Developer ID Application/ { print $2; exit }')"
  if [[ -z "$identity" ]]; then
    identity="$(security find-identity -v -p codesigning 2>/dev/null | awk '/Apple Development/ { print $2; exit }')"
  fi
  if [[ -n "$identity" ]]; then
    echo "Signing with ${identity}..."
    if ! codesign --force --deep --sign "$identity" --identifier local.file-transfer "$app"; then
      echo "warning: codesign failed; Local Network permission may not stick" >&2
    fi
    return
  fi
  echo "Ad-hoc signing (no Apple code-signing identity found)..."
  if ! codesign --force --deep --sign - --identifier local.file-transfer "$app"; then
    echo "warning: ad-hoc codesign failed; Local Network permission may not stick" >&2
  fi
}

echo "Building release binary..."
cargo build --release -p ft-app

BIN="$CARGO_TARGET_DIR/release/ft-app"
APP_DIR="$CARGO_TARGET_DIR/release/bundle/osx/File Transfer.app"
CONTENTS="$APP_DIR/Contents"
MACOS="$CONTENTS/MacOS"
RES="$CONTENTS/Resources"

rm -rf "$APP_DIR"
mkdir -p "$MACOS" "$RES"
cp "$BIN" "$MACOS/file-transfer"
chmod +x "$MACOS/file-transfer"

"$ROOT/scripts/make-app-icon.sh"
ICON="$ROOT/crates/ft-app/assets/AppIcon.icns"
if [[ ! -f "$ICON" ]]; then
  echo "Missing app icon: $ICON" >&2
  exit 1
fi
cp "$ICON" "$RES/AppIcon.icns"

# cargo pkgid is `path+file:///…/ft-app#1.2.3` (or older `…@1.2.3`), not a bare version.
pkgid="$(cargo pkgid -p ft-app)"
APP_VERSION="${pkgid##*#}"
APP_VERSION="${APP_VERSION##*@}"

cat > "$CONTENTS/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleExecutable</key>
  <string>file-transfer</string>
  <key>CFBundleIdentifier</key>
  <string>local.file-transfer</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>File Transfer</string>
  <key>CFBundleDisplayName</key>
  <string>File Transfer</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>${APP_VERSION}</string>
  <key>CFBundleVersion</key>
  <string>${APP_VERSION}</string>
  <key>NSHumanReadableCopyright</key>
  <string>Copyright © 2026 Steve</string>
  <key>CFBundleIconFile</key>
  <string>AppIcon</string>
  <key>LSMinimumSystemVersion</key>
  <string>13.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSQuitAlwaysKeepsWindows</key>
  <false/>
  <key>NSLocalNetworkUsageDescription</key>
  <string>File Transfer discovers SSH hosts on your local network and copies files to them.</string>
  <key>NSBonjourServices</key>
  <array>
    <string>_ssh._tcp</string>
  </array>
</dict>
</plist>
PLIST

echo "Assembled: $APP_DIR"

DEST="/Applications/File Transfer.app"
echo "Installing to ${DEST}..."
rm -rf "$DEST"
cp -R "$APP_DIR" "$DEST"
touch "$DEST"
sign_app "$DEST"
echo "Installed to ${DEST}"
echo "On first launch, allow File Transfer under System Settings → Privacy & Security → Local Network."
