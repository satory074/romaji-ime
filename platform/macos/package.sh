#!/usr/bin/env bash
# Build a distributable RomajiIME installer (.pkg) for macOS.
#
# Builds the universal app (signed if CODESIGN_IDENTITY is set), wraps it in a
# component + distribution .pkg that installs to /Library/Input Methods, signs
# the installer (if INSTALLER_IDENTITY is set), and notarizes + staples it (if
# NOTARY_PROFILE is set). Anything unset is skipped with a warning, so this also
# produces an *unsigned* .pkg for local testing.
#
# One-time setup (with an Apple Developer account):
#   1. In Xcode/developer portal, create & install these certs in your keychain:
#        - "Developer ID Application: NAME (TEAMID)"
#        - "Developer ID Installer: NAME (TEAMID)"
#      Check with: security find-identity -v
#   2. Store notarization credentials once:
#        xcrun notarytool store-credentials romaji-ime-notary \
#          --apple-id you@example.com --team-id TEAMID --password <app-specific-password>
#   3. Run:
#        CODESIGN_IDENTITY="Developer ID Application: NAME (TEAMID)" \
#        INSTALLER_IDENTITY="Developer ID Installer: NAME (TEAMID)" \
#        NOTARY_PROFILE=romaji-ime-notary \
#        platform/macos/package.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD="$SCRIPT_DIR/build"
APP="$BUILD/RomajiIME.app"
PKG_ID="com.satory074.inputmethod.RomajiIME"
VERSION="0.1.0"
INSTALL_LOCATION="/Library/Input Methods"

echo ">> Building app (CODESIGN_IDENTITY=${CODESIGN_IDENTITY:-<ad-hoc>})"
CODESIGN_IDENTITY="${CODESIGN_IDENTITY:-}" "$SCRIPT_DIR/build.sh"

echo ">> Staging payload"
PKGROOT="$BUILD/pkgroot"
rm -rf "$PKGROOT"
mkdir -p "$PKGROOT"
ditto "$APP" "$PKGROOT/RomajiIME.app"  # bundle-correct copy (no ._ AppleDouble files)

echo ">> pkgbuild (component) -> installs to $INSTALL_LOCATION"
pkgbuild --root "$PKGROOT" --install-location "$INSTALL_LOCATION" \
    --identifier "$PKG_ID" --version "$VERSION" \
    "$BUILD/RomajiIME-component.pkg"

echo ">> productbuild (distribution)"
if [ -n "${INSTALLER_IDENTITY:-}" ]; then
    productbuild --package "$BUILD/RomajiIME-component.pkg" \
        --sign "$INSTALLER_IDENTITY" "$BUILD/RomajiIME.pkg"
else
    echo "   (INSTALLER_IDENTITY unset -> unsigned installer)"
    productbuild --package "$BUILD/RomajiIME-component.pkg" "$BUILD/RomajiIME.pkg"
fi

if [ -n "${NOTARY_PROFILE:-}" ]; then
    echo ">> Notarizing (notarytool, profile=$NOTARY_PROFILE)"
    xcrun notarytool submit "$BUILD/RomajiIME.pkg" --keychain-profile "$NOTARY_PROFILE" --wait
    echo ">> Stapling"
    xcrun stapler staple "$BUILD/RomajiIME.pkg"
else
    echo ">> NOTARY_PROFILE unset -> skipping notarization (Gatekeeper will warn on other Macs)"
fi

echo ">> Done: $BUILD/RomajiIME.pkg"
