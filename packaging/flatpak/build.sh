#!/usr/bin/env sh
# Generates cargo-sources.json from Cargo.lock and builds the Flatpak into a work directory.
#   packaging/flatpak/build.sh [WORKDIR] [--install]
# WORKDIR defaults to ~/.cache/kelma-flatpak. Keep it on a disk with a few GB free (not tmpfs).
set -eu

# flatpak-builder-tools commit the generator is fetched from (pin it, it runs on your machine).
GEN_REV=9d86d336a39b9af38b86f114b6ce5820fe260f91
APP_ID=io.github.mohamedeslam04.kelma

here=$(cd "$(dirname "$0")" && pwd)
work=${1:-${XDG_CACHE_HOME:-$HOME/.cache}/kelma-flatpak}
mkdir -p "$work"
work=$(cd "$work" && pwd)
install_flag=
[ "${2:-}" = "--install" ] && install_flag=--install

if [ ! -x "$work/venv/bin/python3" ]; then
  python3 -m venv "$work/venv"
  "$work/venv/bin/pip" -q install 'aiohttp>=3.9.5,<4' 'PyYAML>=6.0.2,<7' 'tomlkit>=0.13.3,<1'
fi
if [ ! -f "$work/flatpak-cargo-generator.py" ]; then
  curl -fsSL -o "$work/flatpak-cargo-generator.py" \
    "https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/$GEN_REV/cargo/flatpak-cargo-generator.py"
fi
"$work/venv/bin/python3" -I "$work/flatpak-cargo-generator.py" "$here/../../Cargo.lock" \
  -o "$here/cargo-sources.json"

if command -v flatpak-builder >/dev/null 2>&1; then
  builder=flatpak-builder
else
  builder="flatpak run org.flatpak.Builder"
fi
# One build job by default (the manifest sets CARGO_BUILD_JOBS=1 too): the compile needs several GB
# of memory per job. Raise it with KELMA_FLATPAK_JOBS=4 on a bigger machine.
# shellcheck disable=SC2086
$builder --user $install_flag --force-clean --install-deps-from=flathub \
  --jobs="${KELMA_FLATPAK_JOBS:-1}" \
  --state-dir="$work/state" "$work/build-dir" "$here/$APP_ID.yml"
