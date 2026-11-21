#!/usr/bin/env bash
# Renders packaging/io.github.mohamedeslam04.kelma.svg into the icons the Windows and macOS
# packages need: <dir>/kelma.ico, <dir>/kelma.icns and PNGs. Needs rsvg-convert (librsvg2-bin),
# ImageMagick and png2icns (icnsutils).
set -euo pipefail
dir=${1:?usage: icons.sh <dir>}
svg="$(dirname "$0")/../io.github.mohamedeslam04.kelma.svg"
mkdir -p "$dir"
for s in 16 24 32 48 64 128 256 512 1024; do
  rsvg-convert -w "$s" -h "$s" "$svg" -o "$dir/kelma-$s.png"
done
convert "$dir"/kelma-{16,24,32,48,64,128,256}.png "$dir/kelma.ico"
png2icns "$dir/kelma.icns" "$dir"/kelma-{16,32,48,128,256,512,1024}.png
