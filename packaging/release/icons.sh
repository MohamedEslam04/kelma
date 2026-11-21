#!/usr/bin/env bash
# Renders the app's SVG icon into the icons the Windows and macOS packages need: <dir>/app.ico,
# <dir>/app.icns and PNGs. Needs rsvg-convert (librsvg2-bin), ImageMagick and png2icns
# (icnsutils).
set -euo pipefail
dir=${1:?usage: icons.sh <dir> <icon.svg>}
svg=${2:?usage: icons.sh <dir> <icon.svg>}
mkdir -p "$dir"
for s in 16 24 32 48 64 128 256 512 1024; do
  rsvg-convert -w "$s" -h "$s" "$svg" -o "$dir/app-$s.png"
done
convert "$dir"/app-{16,24,32,48,64,128,256}.png "$dir/app.ico"
png2icns "$dir/app.icns" "$dir"/app-{16,32,48,128,256,512,1024}.png
