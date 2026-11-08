#!/bin/sh
# Zips the built Mac app for a GitHub Release into release/. Run after `cargo tauri build`
# on a Mac, then put the zip in the Windows checkout's release/ before `npm run dist`,
# which writes one checksum file for both.
set -eu
cd "$(dirname "$0")/.."
# Reads the first "version", whether package.json ends its lines with LF or CRLF.
version=$(sed -n 's/^  "version": *"\([0-9.]*\)".*/\1/p' package.json | head -n 1)
if [ -z "$version" ]; then
  echo "Could not read the version from package.json" >&2
  exit 1
fi
app=src-tauri/target/release/bundle/macos/YuiLauncher.app
name="YuiLauncher-$version-mac-arm64.zip"
mkdir -p release
rm -f "release/$name"
# ditto keeps the bundle's signature and symlinks intact, which zip does not.
ditto -c -k --keepParent "$app" "release/$name"
echo "Packaged release/$name"
shasum -a 256 "release/$name"
