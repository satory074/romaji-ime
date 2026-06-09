#!/usr/bin/env bash
# Build RomajiIME.app (macOS InputMethodKit input method).
#
# Compiles the Rust engine to a universal cdylib, builds the Swift sources with
# swiftc (no Xcode project needed), assembles the .app bundle, and ad-hoc signs
# it. Pass --install to copy it into ~/Library/Input Methods/.
#
#   ./build.sh              # build a universal RomajiIME.app under build/
#   ./build.sh --install    # build, then install to ~/Library/Input Methods/
#   ARCHS="arm64" ./build.sh # build host-arch only (faster for local testing)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
SRC="$SCRIPT_DIR/Sources/RomajiIME"
INCLUDE_DIR="$ROOT/crates/ime-ffi/include"
BUILD="$SCRIPT_DIR/build"
APP="$BUILD/RomajiIME.app"
ARCHS="${ARCHS:-arm64 x86_64}"
DEPLOY_TARGET="13.0"

# Map a build arch to its Rust target triple.
rust_target() { case "$1" in arm64) echo "aarch64-apple-darwin";; x86_64) echo "x86_64-apple-darwin";; esac; }

echo ">> Building Rust cdylib (release) for: $ARCHS"
DYLIBS=()
for arch in $ARCHS; do
    target="$(rust_target "$arch")"
    ( cd "$ROOT" && cargo build --release -p ime-ffi --target "$target" >/dev/null )
    DYLIBS+=("$ROOT/target/$target/release/libime_ffi.dylib")
done

echo ">> Assembling app bundle"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Frameworks" "$APP/Contents/Resources"
cp "$SCRIPT_DIR/Resources/Info.plist" "$APP/Contents/Info.plist"

# Universal dylib in Frameworks, with an @rpath install name.
lipo -create "${DYLIBS[@]}" -output "$APP/Contents/Frameworks/libime_ffi.dylib"
install_name_tool -id "@rpath/libime_ffi.dylib" "$APP/Contents/Frameworks/libime_ffi.dylib"

echo ">> Compiling Swift ($ARCHS)"
EXES=()
for arch in $ARCHS; do
    out="$BUILD/RomajiIME-$arch"
    xcrun swiftc \
        -swift-version 5 \
        -module-name RomajiIME \
        -target "${arch}-apple-macos${DEPLOY_TARGET}" \
        -import-objc-header "$SRC/Bridging-Header.h" \
        -I "$INCLUDE_DIR" \
        -L "$APP/Contents/Frameworks" -lime_ffi \
        -Xlinker -rpath -Xlinker "@executable_path/../Frameworks" \
        -framework InputMethodKit -framework Cocoa \
        -O \
        "$SRC"/*.swift \
        -o "$out"
    EXES+=("$out")
done
lipo -create "${EXES[@]}" -output "$APP/Contents/MacOS/RomajiIME"
rm -f "${EXES[@]}"

echo ">> Ad-hoc code signing"
codesign --force --sign - "$APP/Contents/Frameworks/libime_ffi.dylib"
codesign --force --sign - "$APP"

echo ">> Built: $APP"
file "$APP/Contents/MacOS/RomajiIME"

if [[ "${1:-}" == "--install" ]]; then
    DEST="$HOME/Library/Input Methods"
    echo ">> Installing to $DEST"
    mkdir -p "$DEST"
    rm -rf "$DEST/RomajiIME.app"
    cp -R "$APP" "$DEST/RomajiIME.app"
    echo ">> Installed. Enable it in System Settings > Keyboard > Text Input >"
    echo "   Input Sources > Edit… > + > Japanese > RomajiIME, then switch to it"
    echo "   and type e.g. 'konnichiha' in TextEdit."
fi
