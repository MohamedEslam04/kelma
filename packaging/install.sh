#!/usr/bin/env sh
# Builds ironteams and installs it for the current user (~/.local).
set -eu
cd "$(dirname "$0")/.."
ID=io.github.mohamedeslam04.ironteams
share="${XDG_DATA_HOME:-$HOME/.local/share}"
bin="$HOME/.local/bin/ironteams"
cargo build --release -p ironteams
install -Dm755 target/release/ironteams "$bin"
# The desktop portal and systemd's autostart run with PATH=/usr/local/bin:/usr/bin:/bin, and GLib
# skips a desktop file whose Exec program is not on PATH ("App info not found" when ironteams
# registers with the portal). So a per-user install names the binary by its full path.
exec_path=$bin
case $bin in *[[:space:]]*) exec_path="\"$bin\"" ;; esac
mkdir -p "$share/applications"
sed "s|^Exec=ironteams|Exec=$exec_path|" "packaging/$ID.desktop" > "$share/applications/$ID.desktop"
install -Dm644 "packaging/$ID.svg" "$share/icons/hicolor/scalable/apps/$ID.svg"
install -Dm644 "packaging/$ID.metainfo.xml" "$share/metainfo/$ID.metainfo.xml"
# Older versions of this script installed the desktop file and icon as kelma.*, and before the
# rename to IronTeams the app id was io.github.mohamedeslam04.kelma.
old_id=io.github.mohamedeslam04.kelma
rm -f "$share/applications/kelma.desktop" "$share/icons/hicolor/scalable/apps/kelma.svg" \
  "$share/applications/$old_id.desktop" "$share/icons/hicolor/scalable/apps/$old_id.svg" \
  "$share/metainfo/$old_id.metainfo.xml" "$HOME/.local/bin/kelma"
if command -v update-desktop-database >/dev/null; then update-desktop-database "$share/applications" || true; fi
echo "Installed. Start ironteams from your app launcher or run: $bin"
