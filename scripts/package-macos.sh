#!/bin/sh
# Build a simple drag-to-Applications disk image without mounting a volume or scripting Finder.
set -eu
cd "$(dirname "$0")/.."
npm run package -- --bundles app
stage=$(mktemp -d "${TMPDIR:-/tmp}/allowance-package.XXXXXX")
trap 'rm -rf "$stage"' EXIT HUP INT TERM
mkdir -p "$stage/root" artifacts
cp -R src-tauri/target/release/bundle/macos/Allowance.app "$stage/root/"
xattr -cr "$stage/root/Allowance.app"
codesign --force --sign - --timestamp=none "$stage/root/Allowance.app"
codesign --verify --deep --strict "$stage/root/Allowance.app"
ln -s /Applications "$stage/root/Applications"
output="artifacts/Allowance_macOS-$(uname -m).dmg"
hdiutil create -volname Allowance -srcfolder "$stage/root" -ov -format UDZO "$output"
hdiutil verify "$output"
