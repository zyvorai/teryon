#!/usr/bin/env bash
# Render docs/social/teryon-social-card.html (1600x900) with headless Chrome, then cut the sizes the
# platforms want. Needs Google Chrome and macOS `sips` (both already on a Mac); nothing is installed.
#   ./docs/social/build-social-card.sh
# Writes next to the HTML: teryon-social-card.jpg (1600x900, X and LinkedIn), teryon-og.png (1200x630,
# og:image) and teryon-github-social.png (1280x640, GitHub Settings > Social preview).
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
CHROME="${CHROME:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"
[[ -x "$CHROME" ]] || { echo "Google Chrome not found (set CHROME=...)" >&2; exit 1; }
PNG="$(mktemp "${TMPDIR:-/tmp}/teryon-card.XXXXXX.png")"
CUT="$(mktemp "${TMPDIR:-/tmp}/teryon-cut.XXXXXX.png")"
trap 'rm -f "$PNG" "$CUT"' EXIT
"$CHROME" --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=1 \
  --window-size=1600,900 --screenshot="$PNG" "file://$HERE/teryon-social-card.html" >/dev/null 2>&1
sips -s format jpeg -s formatOptions 92 "$PNG" --out "$HERE/teryon-social-card.jpg" >/dev/null
# 1200x630 is 1.9:1 and 1280x640 is 2:1: crop the 16:9 render around its centre, then scale.
sips -c 840 1600 "$PNG" --out "$CUT" >/dev/null
sips -z 630 1200 "$CUT" --out "$HERE/teryon-og.png" >/dev/null
sips -c 800 1600 "$PNG" --out "$CUT" >/dev/null
sips -z 640 1280 "$CUT" --out "$HERE/teryon-github-social.png" >/dev/null
for f in teryon-social-card.jpg teryon-og.png teryon-github-social.png; do
  echo "wrote $f ($(sips -g pixelWidth -g pixelHeight "$HERE/$f" | awk '/pixel/{printf "%s ", $2}')px, $(du -k "$HERE/$f" | cut -f1) KB)"
done
