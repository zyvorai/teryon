#!/bin/sh
# Regenerate PNG and .icns brand assets from docs/brand/teryon-icon.svg (macOS: needs rsvg-convert, iconutil, Pillow).
set -eu
cd "$(dirname "$0")/../docs/brand"
command -v rsvg-convert >/dev/null || { echo "install librsvg (brew install librsvg)" >&2; exit 1; }
rsvg-convert -w 512 -h 512 teryon-icon.svg -o teryon-icon.png
rsvg-convert -w 1280 teryon-wordmark.svg -o teryon-wordmark.png
set_dir=teryon.iconset
rm -rf "$set_dir" && mkdir "$set_dir"
for s in 16 32 128 256 512; do
  rsvg-convert -w $s -h $s teryon-icon.svg -o "$set_dir/icon_${s}x${s}.png"
  rsvg-convert -w $((s * 2)) -h $((s * 2)) teryon-icon.svg -o "$set_dir/icon_${s}x${s}@2x.png"
done
rsvg-convert -w 128 -h 128 teryon-icon.svg -o /tmp/teryon-128.png
# Raw RGBA for the desktop window icon (include_bytes! in src/desktop.rs); needs Pillow.
python3 -c "from PIL import Image; open('teryon-icon-128.rgba','wb').write(Image.open('/tmp/teryon-128.png').convert('RGBA').tobytes())"
rm -f /tmp/teryon-128.png
iconutil -c icns "$set_dir" -o Teryon.icns
rm -rf "$set_dir"
echo "Wrote docs/brand/teryon-icon.png, teryon-icon-128.rgba, teryon-wordmark.png, Teryon.icns"
