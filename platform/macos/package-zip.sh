#!/usr/bin/env bash
# Assemble a zip you can send to a (Mac) friend: the built RomajiIME.app plus the
# end-user install.sh and INSTALL.md. The friend needs NO developer toolchain —
# they unzip and run ./install.sh, paste their own free Gemini key, done.
#
# This is the "few technical friends, bring-your-own-free-key" distribution path
# (unsigned/ad-hoc). For a notarized double-click .pkg, use package.sh instead.
#
#   ./package-zip.sh            # -> build/RomajiIME-macos.zip
#   ARCHS="arm64" ./package-zip.sh   # host-arch only (smaller/faster)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD="$SCRIPT_DIR/build"
STAGE="$BUILD/RomajiIME-macos"
ZIP="$BUILD/RomajiIME-macos.zip"

echo ">> Building app"
"$SCRIPT_DIR/build.sh"

echo ">> Staging distribution folder"
rm -rf "$STAGE" "$ZIP"
mkdir -p "$STAGE"
ditto "$BUILD/RomajiIME.app" "$STAGE/RomajiIME.app"  # AppleDouble-clean copy
cp "$SCRIPT_DIR/install.sh" "$STAGE/install.sh"
cp "$SCRIPT_DIR/INSTALL.md" "$STAGE/INSTALL.md"
chmod +x "$STAGE/install.sh"

echo ">> Zipping"
# --keepParent without --sequesterRsrc: preserves the code signature and avoids
# the noisy __MACOSX/ AppleDouble tree in the archive.
( cd "$BUILD" && ditto -c -k --keepParent "RomajiIME-macos" "$ZIP" )

echo ">> Done: $ZIP"
echo "   友達に送る → 展開 → INSTALL.md の通り ./install.sh"
