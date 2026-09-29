#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
cargo build --release --locked -j 2
bundle="$PWD/dist/Vyber.app"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp target/release/vyber "$bundle/Contents/MacOS/vyber"
cp assets/vyber.icns "$bundle/Contents/Resources/vyber.icns"
cp assets/Info.plist "$bundle/Contents/Info.plist"
codesign --force --deep --sign - "$bundle"
echo "Built: $bundle"
if [ "${1:-}" = "--install" ]; then
    ditto "$bundle" /Applications/Vyber.app
    echo "Installed: /Applications/Vyber.app"
fi
