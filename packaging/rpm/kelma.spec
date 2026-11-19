# Rust binaries are stripped by the release profile (strip = true), so there is no debuginfo to split.
%global debug_package %{nil}

Name:           kelma
Version:        0.1.0
Release:        1%{?dist}
Summary:        Lightweight native client for Microsoft Teams chat (unofficial)

License:        GPL-3.0-or-later
URL:            https://github.com/MohamedEslam04/kelma
Source0:        %{url}/archive/v%{version}/%{name}-%{version}.tar.gz
# Crates vendored from Cargo.lock, so the build needs no network (see packaging/rpm/README.md):
#   cargo vendor && tar caf kelma-%%{version}-vendor.tar.xz vendor
Source1:        %{name}-%{version}-vendor.tar.xz

# slint 1.19 needs Rust 1.95 or newer.
BuildRequires:  cargo >= 1.95
BuildRequires:  rust >= 1.95
BuildRequires:  gcc
BuildRequires:  gcc-c++
# cmake builds the bundled C libraries (aws-lc, and libopus for the meeting host).
BuildRequires:  cmake
BuildRequires:  pkgconfig
# alsa-lib is for the meeting host's audio (cpal); harmless until that crate lands.
BuildRequires:  pkgconfig(alsa)
BuildRequires:  pkgconfig(fontconfig)
BuildRequires:  pkgconfig(xkbcommon)
BuildRequires:  pkgconfig(dbus-1)
BuildRequires:  desktop-file-utils
Requires:       hicolor-icon-theme
# Loaded with dlopen (winit, glutin, xkbcommon-dl), so rpm does not detect them:
Requires:       libxkbcommon
Requires:       libxkbcommon-x11
Requires:       libwayland-client
Requires:       libwayland-egl
Requires:       libX11
Requires:       libXcursor
Requires:       libXi
Requires:       libXrender
Requires:       libglvnd-egl
Requires:       libglvnd-glx
Recommends:     google-noto-emoji-fonts
Recommends:     xdg-desktop-portal
# Screen sharing in calls runs `gst-launch-1.0 pipewiresrc` on the ScreenCast portal's stream.
Recommends:     gstreamer1
Recommends:     gstreamer1-plugins-base
Recommends:     pipewire-gstreamer
# Camera video in calls: `pipewiresrc` (Camera portal) or `v4l2src`, and `jpegdec` for MJPEG
# cameras, from plugins-good.
Recommends:     gstreamer1-plugins-good

%description
kelma is a small, fast Linux client for Microsoft Teams chat, channels, activity, calendar and
files, built with Rust and Slint. It does not embed a browser and collects no telemetry: the
"Report a problem" action writes a local, redacted file and never uploads it. It is not
affiliated with or endorsed by Microsoft.

%prep
%autosetup -n %{name}-%{version} -a 1
mkdir -p .cargo
cat > .cargo/config.toml <<'EOF'
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
EOF

%build
export CARGO_HOME=%{_builddir}/cargo-home
# No -debuginfo package is built (see debug_package above), and the default -Cdebuginfo=2 makes the
# kelma crate (a lot of Slint-generated code) need more memory than a 7 GB CI runner has.
export RUSTFLAGS="${RUSTFLAGS:-}"
RUSTFLAGS="${RUSTFLAGS//-Cdebuginfo=2/-Cdebuginfo=0}"
cargo build --release --frozen -p kelma

%check
export CARGO_HOME=%{_builddir}/cargo-home
desktop-file-validate packaging/kelma.desktop
cargo test --frozen -p diag

%install
install -Dm755 target/release/kelma %{buildroot}%{_bindir}/kelma
install -Dm644 packaging/kelma.desktop %{buildroot}%{_datadir}/applications/kelma.desktop
install -Dm644 packaging/kelma.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/kelma.svg

%files
%license LICENSE
%doc README.md
%{_bindir}/kelma
%{_datadir}/applications/kelma.desktop
%{_datadir}/icons/hicolor/scalable/apps/kelma.svg

%changelog
* Thu Nov 19 2026 kelma contributors - 0.1.0-1
- First package
