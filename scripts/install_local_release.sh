#!/bin/zsh
set -euo pipefail

repo="/Users/deepsaint/Desktop/aegis"
release_bin="$repo/target/aarch64-apple-darwin/release/aegis"
release_app="$repo/native/build-xcode/Release/aegis_native.app"
install_root="$HOME/Applications"
installed_app="$install_root/Aegis.app"

cd "$repo"

cargo build --release
"$release_bin" native build --configuration release --scheme aegis_host >/dev/null
"$release_bin" native build --configuration release >/dev/null

mkdir -p "$install_root"
rm -rf "$installed_app"
cp -R "$release_app" "$installed_app"
cp "$release_bin" "$installed_app/Contents/MacOS/aegis_cli"
chmod +x "$installed_app/Contents/MacOS/aegis_cli"
xattr -cr "$installed_app" || true
codesign --force --deep --sign - "$installed_app"

printf '%s\n' "$installed_app"
