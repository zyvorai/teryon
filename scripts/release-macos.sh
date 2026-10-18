#!/bin/sh
# Build, sign and notarize TERYON.app, then wrap it in a .dmg and a .pkg.
# Run on the Mac mini. The .pkg step needs the GUI session (see docs/DEPLOYMENT.md).
# Required environment (keep these out of the repo):
#   MACORA_SIGN_IDENTITY   "Developer ID Application: ..." identity name
#   DEVELOPER_ID_INSTALLER "Developer ID Installer: ..." identity name
#   NOTARY_PROFILE         notarytool keychain profile name
set -eu
test "$(uname -s)" = Darwin || { echo "Run this on macOS" >&2; exit 1; }
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

APP_ID="${MACORA_SIGN_IDENTITY:?set MACORA_SIGN_IDENTITY}"
PKG_ID="${DEVELOPER_ID_INSTALLER:?set DEVELOPER_ID_INSTALLER}"
PROFILE="${NOTARY_PROFILE:?set NOTARY_PROFILE}"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
app=dist/TERYON.app
dmg="dist/TERYON-$version.dmg"
pkg="dist/TERYON-$version.pkg"

rm -rf "$app" "$dmg" "$pkg"
scripts/bundle-macos.sh

codesign --force --options runtime --timestamp --sign "$APP_ID" "$app/Contents/MacOS/teryon"
codesign --force --options runtime --timestamp --sign "$APP_ID" "$app"
codesign --verify --strict --verbose=2 "$app"

# DMG: the app plus an Applications link.
stage=$(mktemp -d)
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
hdiutil create -volname "TERYON $version" -srcfolder "$stage" -ov -format UDZO "$dmg"
rm -rf "$stage"
codesign --force --timestamp --sign "$APP_ID" "$dmg"
xcrun notarytool submit "$dmg" --keychain-profile "$PROFILE" --wait
xcrun stapler staple "$dmg"

# PKG: installs TERYON.app into /Applications.
pkgbuild --component "$app" --install-location /Applications --sign "$PKG_ID" "$pkg"
xcrun notarytool submit "$pkg" --keychain-profile "$PROFILE" --wait
xcrun stapler staple "$pkg"

(cd dist && shasum -a 256 "TERYON-$version.dmg" "TERYON-$version.pkg" > "SHA256SUMS-macos-$version")
echo "Built $dmg and $pkg"
