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

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  gcc
BuildRequires:  pkgconfig
BuildRequires:  pkgconfig(fontconfig)
BuildRequires:  pkgconfig(xkbcommon)
BuildRequires:  pkgconfig(dbus-1)
BuildRequires:  desktop-file-utils
Requires:       hicolor-icon-theme
Recommends:     google-noto-emoji-fonts
Recommends:     xdg-desktop-portal

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
cargo build --release --frozen -p kelma

%check
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
