#!/bin/sh
# Copyright 2026 Zyvor AI Labs · https://zyvor.dev
# SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

set -eu
binary=${1:-./target/release/teryon}
destination=${TERYON_INSTALL_DIR:-"$HOME/.local/bin"}
test -f "$binary" || { echo "Build first or pass an executable path" >&2; exit 1; }
mkdir -p "$destination"
temporary=$(mktemp "$destination/.teryon-install.XXXXXX")
trap 'rm -f "$temporary"' EXIT HUP INT TERM
cp "$binary" "$temporary"
chmod 755 "$temporary"
"$temporary" --version
mv -f "$temporary" "$destination/teryon"
echo "Installed $destination/teryon"
