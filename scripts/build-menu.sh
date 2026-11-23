#!/bin/sh
# Build TeryonMenu.app (the menu-bar item) into $1/Contents/Library (default dist/TERYON.app) and sign it.
# SIGN_IDENTITY is a Developer ID for releases; the default "-" is an ad-hoc signature.
set -eu
test "$(uname -s)" = Darwin || { echo "Run this on macOS" >&2; exit 1; }
cd "$(dirname "$0")/.."
app=${1:-dist/TERYON.app}
helper="$app/Contents/Library/TeryonMenu.app"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
rm -rf "$helper"
mkdir -p "$helper/Contents/MacOS" "$helper/Contents/Resources"
swiftc -O -o "$helper/Contents/MacOS/TeryonMenu" native/TeryonMenu/main.swift
sed "s/__VERSION__/$version/" native/TeryonMenu/Info.plist > "$helper/Contents/Info.plist"
cp docs/brand/Teryon.icns "$helper/Contents/Resources/Teryon.icns"
codesign --force --options runtime --sign "${SIGN_IDENTITY:--}" "$helper"
echo "Built $helper"
