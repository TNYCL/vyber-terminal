#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
target=${1:-x86_64-unknown-linux-gnu}
case "$target" in
  x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu) ;;
  *) echo "Unsupported Linux target: $target" >&2; exit 2 ;;
esac
if [[ ${2:-} != --skip-build ]]; then
  cargo build --release --locked --target "$target"
fi
binary=$(python3 scripts/release.py binary-path --target "$target")
asset=$(python3 scripts/release.py asset-name --target "$target")
mkdir -p dist/release dist/staging
stage=$(mktemp -d "$PWD/dist/staging/linux.XXXXXX")
trap 'rm -rf -- "$stage"' EXIT
folder="$stage/${asset%.tar.gz}"
python3 scripts/release.py stage --target "$target" --directory "$folder"
mkdir -p "$folder/bin" "$folder/share/applications" "$folder/share/icons/hicolor/256x256/apps"
install -m 755 "$binary" "$folder/bin/vyber"
install -m 644 assets/dev.vyber.terminal.desktop "$folder/share/applications/"
install -m 644 assets/vyber.png "$folder/share/icons/hicolor/256x256/apps/dev.vyber.terminal.png"
ldd "$binary" | tee "$folder/runtime-libraries.txt"
if grep -q 'not found' "$folder/runtime-libraries.txt"; then
  echo 'Missing Linux runtime libraries' >&2
  exit 1
fi
tar -czf "dist/release/$asset" -C "$stage" "${asset%.tar.gz}"
python3 scripts/release.py record --target "$target"
echo "Packaged: dist/release/$asset"
