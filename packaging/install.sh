#!/usr/bin/env sh
# Builds kelma and installs it for the current user (~/.local).
set -eu
cd "$(dirname "$0")/.."
ID=io.github.mohamedeslam04.kelma
share="${XDG_DATA_HOME:-$HOME/.local/share}"
bin="$HOME/.local/bin/kelma"
cargo build --release -p kelma
install -Dm755 target/release/kelma "$bin"
# The desktop portal and systemd's autostart run with PATH=/usr/local/bin:/usr/bin:/bin, and GLib
# skips a desktop file whose Exec program is not on PATH ("App info not found" when kelma
# registers with the portal). So a per-user install names the binary by its full path.
exec_path=$bin
case $bin in *[[:space:]]*) exec_path="\"$bin\"" ;; esac
mkdir -p "$share/applications"
sed "s|^Exec=kelma|Exec=$exec_path|" "packaging/$ID.desktop" > "$share/applications/$ID.desktop"
install -Dm644 "packaging/$ID.svg" "$share/icons/hicolor/scalable/apps/$ID.svg"
install -Dm644 "packaging/$ID.metainfo.xml" "$share/metainfo/$ID.metainfo.xml"
# Older versions of this script installed the desktop file and icon as kelma.*.
rm -f "$share/applications/kelma.desktop" "$share/icons/hicolor/scalable/apps/kelma.svg"
if command -v update-desktop-database >/dev/null; then update-desktop-database "$share/applications" || true; fi
echo "Installed. Start kelma from your app launcher or run: $bin"
