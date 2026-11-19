# Packaging

| Format | Folder | Status |
|---|---|---|
| Flatpak | `flatpak/` | see the status table at the end |
| RPM (Fedora) | `rpm/` | see the status table at the end |
| AUR (`kelma-git`) | `aur/` | see the status table at the end |
| Per-user install script | `install.sh` | builds with cargo, installs to `~/.local` |

Shared files: `kelma.desktop` (with a "Report a problem" action) and `kelma.svg` (icon). The
Flatpak ships its own AppStream file, `flatpak/io.github.mohamedeslam04.kelma.metainfo.xml`.

## Native build dependencies

| Need | Why | Fedora | Arch | Freedesktop SDK |
|---|---|---|---|---|
| Rust 1.95+ (slint 1.19) | everything | `cargo rust` | `cargo` | `rust-stable` extension |
| C compiler, C++ compiler | bundled C code (aws-lc) | `gcc gcc-c++` | `gcc` | included |
| cmake | bundled C libraries (aws-lc, and libopus for the meeting host) | `cmake` | `cmake` | included |
| fontconfig | text rendering (`yeslogic-fontconfig-sys`) | `fontconfig-devel` | `fontconfig` | included |
| xkbcommon | keyboard input (Slint winit backend) | `libxkbcommon-devel` | `libxkbcommon` | included |
| dbus | Secret Service, notifications | `dbus-devel` | `dbus` | included |
| alsa-lib | meeting host audio (not linked yet) | `alsa-lib-devel` | `alsa-lib` | included |
| pkg-config | finds the above | `pkgconf-pkg-config` | `pkgconf` | included |

At run time the app needs a Secret Service provider (GNOME Keyring or KWallet) to keep the
sign-in, and benefits from `xdg-desktop-portal` and an emoji font.

## Flatpak

Needs `flatpak`, plus either `flatpak-builder` or the Flatpak `org.flatpak.Builder`
(`flatpak install --user flathub org.flatpak.Builder`). The runtime, SDK and `rust-stable` extension
(Freedesktop 26.08) are installed by the script when missing, roughly 2 to 3 GB.

```sh
packaging/flatpak/build.sh /data/kelma-flatpak          # build only
packaging/flatpak/build.sh /data/kelma-flatpak --install # build and install for this user
flatpak run io.github.mohamedeslam04.kelma
```

The work directory holds the builder cache and the build tree, so keep it on a disk with free
space, not on tmpfs. The script generates `cargo-sources.json` (the vendored crate list, git-ignored)
from `Cargo.lock` with `flatpak-cargo-generator.py` pinned to a commit of
`flatpak/flatpak-builder-tools`, then runs the builder with no network inside the sandbox. See
`flatpak/README.md` for the permissions and why each one is there.

## RPM

`rpm/kelma.spec` builds from a source tarball plus a tarball of vendored crates, so the build runs
without network (as Koji and COPR do). To build locally without root:

```sh
top=/data/kelma-rpm; v=0.1.0
mkdir -p $top/{SOURCES,BUILD,RPMS,SRPMS,BUILDROOT} /data/kelma-vendor
git archive --prefix=kelma-$v/ -o $top/SOURCES/kelma-$v.tar.gz HEAD
cargo vendor --locked /data/kelma-vendor/vendor
tar -C /data/kelma-vendor -c vendor | xz -T0 > $top/SOURCES/kelma-$v-vendor.tar.xz
rpmbuild -bb --define "_topdir $top" packaging/rpm/kelma.spec
```

The RPM lands in `$top/RPMS/x86_64/`. `rpmbuild` refuses to start when a `BuildRequires` package is
missing (`sudo dnf builddep packaging/rpm/kelma.spec` installs them). If you use rustup instead of
the Fedora `cargo` and `rust` packages, add `--nodeps`. The `%check` section runs
`desktop-file-validate` and the `diag` crate tests.

## AUR

`aur/PKGBUILD` builds `kelma-git` from the repository head. On Arch:

```sh
cd packaging/aur && makepkg -si
```

Regenerate `.SRCINFO` after changing the PKGBUILD with `makepkg --printsrcinfo > .SRCINFO`.
On other distributions, `packaging/aur/` can be checked in an Arch container, see below.

## Checking the AUR package without Arch

With rootless podman (no sudo), build `kelma-git` from a local clone of the repository:

```sh
git clone --bare . /data/kelma-src.git
podman run --rm -v /data/kelma-src.git:/src.git:ro,Z -v $PWD/packaging/aur:/pkg:ro,Z \
  docker.io/library/archlinux:base-devel bash -c '
    pacman -Syu --noconfirm --needed git rust cmake alsa-lib fontconfig libxkbcommon \
      libxkbcommon-x11 dbus wayland libx11 libxcursor libxi libxrender libglvnd hicolor-icon-theme namcap
    useradd -m builder; mkdir /build; cp /pkg/PKGBUILD /pkg/.SRCINFO /build; chown builder /build; cd /build
    sed -i "s|git+\${url}.git|git+file:///src.git#branch=<your branch>|" PKGBUILD
    git config --system --add safe.directory "*"
    su builder -c "makepkg -f --noconfirm" && namcap PKGBUILD *.pkg.tar.zst'
```

It takes about 30 minutes and several GB of memory (limit cargo with `CARGO_BUILD_JOBS` if the OOM killer steps in). The container's `makepkg` also confirms `.SRCINFO`
(`makepkg --printsrcinfo | diff - .SRCINFO`, after undoing the `sed`).

## CI

`.github/workflows/packaging.yml` builds the RPM (Fedora container), the Flatpak (Flathub's
`flatpak-github-actions` image, Freedesktop 26.08) and the AUR package (Arch container). It never
runs on pull requests. It runs when started by hand (Actions tab), when `packaging/`, `Cargo.lock`
or the workflow change on `main`, and on a push to any branch named `ci/packaging-*`. A manual run
only works once the workflow is on the default branch, so to test a change to it from a branch:

```sh
git switch -c ci/packaging-test
date > packaging/.ci-trigger && git add -f packaging/.ci-trigger && git commit -m "ci: run packaging"
git push -u origin ci/packaging-test
gh run watch "$(gh run list --workflow=packaging.yml --branch ci/packaging-test --limit 1 --json databaseId -q '.[0].databaseId')"
gh run view <id> --log-failed        # when a job failed
git push origin --delete ci/packaging-test
```

The throwaway commit matters: the `paths` filter only sees files changed by commits that are new to
the repository, so pushing a branch whose commits are all on GitHub already starts nothing.

## Verification status

Last checked on 2026-11-19 (Fedora 44 host, Freedesktop 26.08, Arch container, GitHub Actions). Local
builds from commit `d31bc90`, CI from the `ci/packaging-test` run
[44516901149](https://github.com/MohamedEslam04/kelma/actions/runs/44516901149) (all three jobs green).

| Package | Result |
|---|---|
| Flatpak | Built with `build.sh` at `d31bc90` (Freedesktop 25.08, offline crate build, 1485 vendored sources). The manifest is now on 26.08 (`flatpak-builder-lint manifest` is clean) and builds in CI on the `freedesktop-26.08` image, with one job (`CARGO_BUILD_JOBS`). The metainfo has three screenshots (`docs/screenshots/appearance-*.png`, demo data); `appstreamcli validate --pedantic` passes offline and warns only that the images are not at the public URL yet (they resolve once the branch is on `main`). Not built locally on 26.08 and not launched on a desktop: the 26.08 runtime download was still running when local builds were paused. |
| RPM | `rpmbuild -bb` without `--nodeps` passes in CI (Fedora container with the `cargo` and `rust` RPMs, so the `BuildRequires` are real). Locally the `BuildRequires` are installed (`dnf builddep`, Fedora cargo/rust 1.99), but the local build was paused. At `d31bc90` the build with `--nodeps` gave a 15 MB `kelma-0.1.0-1.fc44.x86_64.rpm` and the 27 `diag` tests passed. The first CI run was killed by the OOM killer (`rustc` SIGKILL): Fedora's `RUSTFLAGS` has `-Cdebuginfo=2`, so the spec now sets it to 0 (no `-debuginfo` package is built anyway). `rpmlint` on the spec: no errors, one warning (`invalid-url Source1`, the vendor tarball is a local file by design). `rpmlint` on the built RPM not run yet. |
| AUR | `kelma-git` built in an `archlinux:base-devel` podman container with `makepkg` (17 MiB package, 27 `diag` tests passed) and in CI (`makepkg` plus the `.SRCINFO` check). `namcap PKGBUILD` is clean. The `depends` entries for the dlopen-loaded libraries show as "may not be needed" in `namcap`, and are intentional. |
