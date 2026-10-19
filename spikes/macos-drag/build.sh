#!/bin/zsh
set -euo pipefail
cd "${0:A:h}"

APP=build/OrbitSpike.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp Info.plist "$APP/Contents/"

swiftc -swift-version 5 -O \
  -target "$(uname -m)-apple-macos14.0" \
  -framework AppKit -framework UniformTypeIdentifiers \
  -o "$APP/Contents/MacOS/OrbitSpike" Sources/*.swift

codesign --force --sign - "$APP"
echo "Gebaut: $PWD/$APP"
