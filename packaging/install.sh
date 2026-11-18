#!/usr/bin/env sh
# Builds kelma and installs it for the current user (~/.local).
set -eu
cd "$(dirname "$0")/.."
cargo build --release -p kelma
install -Dm755 target/release/kelma "$HOME/.local/bin/kelma"
install -Dm644 packaging/kelma.desktop "$HOME/.local/share/applications/kelma.desktop"
install -Dm644 packaging/kelma.svg "$HOME/.local/share/icons/hicolor/scalable/apps/kelma.svg"
command -v update-desktop-database >/dev/null && update-desktop-database "$HOME/.local/share/applications" || true
echo "Installed. Start kelma from your app launcher or run: kelma"
