#!/usr/bin/env bash
# Build AppIcon.icns from AppIcon.png via a complete macOS .iconset.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PNG="$ROOT/crates/ft-app/assets/AppIcon.png"
OUT="$ROOT/crates/ft-app/assets/AppIcon.icns"

if [[ ! -f "$PNG" ]]; then
  echo "Missing source icon: $PNG" >&2
  exit 1
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
ICONSET="$TMP/AppIcon.iconset"
mkdir -p "$ICONSET"

# filename -> pixel size. @2x is 2× the point size.
write_size() {
  local name="$1" px="$2"
  sips -s format png -z "$px" "$px" "$PNG" --out "$ICONSET/$name" >/dev/null
}

write_size "icon_16x16.png" 16
write_size "icon_16x16@2x.png" 32
write_size "icon_32x32.png" 32
write_size "icon_32x32@2x.png" 64
write_size "icon_128x128.png" 128
write_size "icon_128x128@2x.png" 256
write_size "icon_256x256.png" 256
write_size "icon_256x256@2x.png" 512
write_size "icon_512x512.png" 512
write_size "icon_512x512@2x.png" 1024

missing=0
for name in \
  icon_16x16.png icon_16x16@2x.png \
  icon_32x32.png icon_32x32@2x.png \
  icon_128x128.png icon_128x128@2x.png \
  icon_256x256.png icon_256x256@2x.png \
  icon_512x512.png icon_512x512@2x.png
do
  if [[ ! -f "$ICONSET/$name" ]]; then
    echo "iconset missing $name" >&2
    missing=1
  fi
done
if [[ "$missing" -ne 0 ]]; then
  exit 1
fi

iconutil -c icns -o "$OUT" "$ICONSET"
echo "Wrote $OUT"
