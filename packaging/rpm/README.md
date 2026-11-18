# RPM

`kelma.spec` builds from the release tarball plus a tarball of vendored crates, so the build runs
without network (as Fedora's Koji and COPR do).

```sh
VERSION=0.1.0
git archive --prefix=kelma-$VERSION/ -o ~/rpmbuild/SOURCES/kelma-$VERSION.tar.gz HEAD
cargo vendor --locked ~/rpmbuild/vendor
tar -C ~/rpmbuild -caf ~/rpmbuild/SOURCES/kelma-$VERSION-vendor.tar.xz vendor
rpmbuild -ba packaging/rpm/kelma.spec
```

Build headers needed: `fontconfig-devel libxkbcommon-devel dbus-devel` (pulled in by the
`BuildRequires`). Lint with `rpmspec -P packaging/rpm/kelma.spec` and `rpmlint`.
