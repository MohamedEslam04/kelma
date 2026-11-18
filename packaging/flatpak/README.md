# Flatpak

`io.github.mohamedeslam04.kelma.yml` builds kelma against the Freedesktop 25.08 runtime with the
`rust-stable` SDK extension.

## Permissions

| Permission | Why |
|---|---|
| `--share=network` | Teams, Graph and OneDrive |
| `--socket=wayland`, `--socket=fallback-x11`, `--share=ipc` | window system |
| `--device=dri` | optional GPU renderer (`KELMA_RENDERER=femtovg`) |
| `--socket=pulseaudio`, `--filesystem=xdg-run/pipewire-0` | audio and PipeWire for the future meeting host |
| `--talk-name=org.freedesktop.Notifications` | desktop notifications |
| `--talk-name=org.freedesktop.secrets` | the refresh token is kept in the Secret Service |
| `--filesystem=xdg-download` | saved attachments and problem reports |

The file chooser, open-URI, camera and screen-cast portals need no permission.

## Build

The sandbox has no network during the build, so the crates are vendored from `Cargo.lock`. The
generated `cargo-sources.json` is not committed.

```sh
flatpak install flathub org.freedesktop.Platform//25.08 org.freedesktop.Sdk//25.08 \
  org.freedesktop.Sdk.Extension.rust-stable//25.08
# flatpak-cargo-generator.py comes from https://github.com/flatpak/flatpak-builder-tools (cargo/)
python3 flatpak-cargo-generator.py ../../Cargo.lock -o cargo-sources.json
flatpak-builder --user --install --force-clean build-dir io.github.mohamedeslam04.kelma.yml
flatpak run io.github.mohamedeslam04.kelma
```

Check the metadata with `appstreamcli validate --no-net io.github.mohamedeslam04.kelma.metainfo.xml`.
