#!/usr/bin/env bash
# Downloads the Hunspell dictionaries the Windows and macOS packages ship (English (US) and
# Arabic, from LibreOffice's dictionaries at a fixed tag) into <dir>, with their licences.
# Linux packages use the distribution's hunspell-en / hunspell-ar instead.
set -euo pipefail
dir=${1:?usage: dictionaries.sh <dir>}
tag=libreoffice-25.2.0.1
base=https://raw.githubusercontent.com/LibreOffice/dictionaries/$tag
mkdir -p "$dir"
for f in en/en_US.aff en/en_US.dic en/README_en_US.txt ar/ar.aff ar/ar.dic ar/COPYING.txt; do
  out=$(basename "$f")
  [ "$out" = COPYING.txt ] && out=COPYING_ar.txt
  curl -fsSL --retry 3 -o "$dir/$out" "$base/$f"
done
