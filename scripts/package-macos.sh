#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
target=${1:-aarch64-apple-darwin}
case "$target" in
  aarch64-apple-darwin|x86_64-apple-darwin) ;;
  *) echo "Unsupported macOS target: $target" >&2; exit 2 ;;
esac
export MACOSX_DEPLOYMENT_TARGET=${MACOSX_DEPLOYMENT_TARGET:-12.0}
if [[ ${2:-} != --skip-build ]]; then
  cargo build --release --locked --target "$target"
fi
binary=$(python3 scripts/release.py binary-path --target "$target")
asset=$(python3 scripts/release.py asset-name --target "$target")
version=$(python3 scripts/release.py version)
display_version=${version%%[-+]*}
mkdir -p dist/release dist/staging
stage=$(mktemp -d "$PWD/dist/staging/macos.XXXXXX")
trap 'rm -rf -- "$stage"' EXIT
bundle="$stage/Vyber.app"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
install -m 755 "$binary" "$bundle/Contents/MacOS/vyber"
cp assets/vyber.icns "$bundle/Contents/Resources/vyber.icns"
cp assets/Info.plist "$bundle/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $display_version" "$bundle/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion ${GITHUB_RUN_NUMBER:-1}" "$bundle/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :LSMinimumSystemVersion $MACOSX_DEPLOYMENT_TARGET" "$bundle/Contents/Info.plist"
python3 scripts/release.py stage --target "$target" --directory "$bundle/Contents/Resources"
codesign --force --sign - "$bundle"
codesign --verify --deep --strict --verbose=2 "$bundle"
otool -L "$bundle/Contents/MacOS/vyber" > "$bundle/Contents/Resources/runtime-libraries.txt"
ln -s /Applications "$stage/Applications"
hdiutil create -volname "Vyber $display_version" -srcfolder "$stage" -format UDZO -ov "dist/release/$asset"
python3 scripts/release.py record --target "$target"
echo "Packaged: dist/release/$asset"
