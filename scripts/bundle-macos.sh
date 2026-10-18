#!/bin/sh
set -eu
test "$(uname -s)" = Darwin || { echo "Run this on macOS" >&2; exit 1; }
cd "$(dirname "$0")/.."
cargo build --release --features desktop --locked
app=dist/TERYON.app
mkdir -p "$app/Contents/MacOS"
cp target/release/teryon "$app/Contents/MacOS/teryon"
cat > "$app/Contents/MacOS/launch-teryon" <<'SH'
#!/bin/sh
exec "$(dirname "$0")/teryon" desktop local
SH
chmod 755 "$app/Contents/MacOS/launch-teryon"
mkdir -p "$app/Contents/Resources"
cp docs/brand/Teryon.icns "$app/Contents/Resources/Teryon.icns"
python3 - "$app" <<'PY'
import plistlib,sys
from pathlib import Path
with (Path(sys.argv[1])/'Contents/Info.plist').open('wb') as f:
 plistlib.dump({'CFBundleIdentifier':'dev.zyvor.teryon','CFBundleName':'TERYON','CFBundleExecutable':'launch-teryon','CFBundleIconFile':'Teryon','CFBundlePackageType':'APPL','CFBundleShortVersionString':'0.7.0','NSHighResolutionCapable':True},f)
PY
echo "Created $app (unsigned; opens a local workspace). Use CLI for remote hosts."
