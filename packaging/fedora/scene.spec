# Scene links its Rust dependencies statically, so this package bundles them:
# the sources are Source0 and the crates Cargo.lock resolved are Source1, both
# produced by scripts/release-tarball.sh. The build is offline, which is what
# %%cargo_prep -v vendor sets up.

%global appid dev.scene.Scene

Name:           scene
Version:        0.1.0
Release:        1%{?dist}
Summary:        Fast, keyboard-first Linux launcher

# Scene itself is MIT. The rest of this expression is every licence carried by
# a bundled crate, as %%{cargo_license_summary} reports it at build time:
#
#   (MIT OR Apache-2.0) AND Unicode-3.0
#   (MIT OR Apache-2.0) AND Unicode-DFS-2016
#   Apache-2.0 AND ISC
#   Apache-2.0 OR ISC OR MIT
#   Apache-2.0 OR MIT
#   Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT
#   BSD-3-Clause
#   CDLA-Permissive-2.0
#   ISC
#   MIT
#   MIT OR Apache-2.0
#   Unlicense OR MIT
#   Zlib
#
# LICENSE.dependencies, installed with the package, is the per-crate breakdown.
License:        MIT AND Apache-2.0 AND ISC AND BSD-3-Clause AND CDLA-Permissive-2.0 AND Unicode-3.0 AND Unicode-DFS-2016 AND Zlib AND (MIT OR Apache-2.0) AND (Apache-2.0 OR ISC OR MIT) AND (Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT) AND (Unlicense OR MIT)
URL:            https://github.com/fullstacktaiye/Scene
Source0:        %{url}/archive/v%{version}/%{name}-%{version}.tar.gz
Source1:        %{name}-%{version}-vendor.tar.gz

ExclusiveArch:  %{rust_arches}

BuildRequires:  cargo-rpm-macros >= 24
BuildRequires:  gcc
BuildRequires:  pkgconfig(gtk4)
BuildRequires:  pkgconfig(sqlite3)
BuildRequires:  desktop-file-utils
BuildRequires:  libappstream-glib

# The icon is Scene's own and installs into the hicolor theme.
Requires:       hicolor-icon-theme

# Nothing else is a hard requirement on purpose. Scene detects the tools it can
# use — a terminal, dnf, pkexec, KDE's shortcut editor, Baloo — and reports the
# missing ones as unavailable capabilities rather than failing to start.

%description
Scene is a keyboard-first launcher for Linux, built as one Rust binary against
native GTK 4 and aimed at KDE Plasma on Wayland first. Activate it, type, and
it ranks installed applications, folders, links, calculations, package queries
and its own commands in one deterministic list.

System actions are typed and bounded rather than free-form shell commands.
Every command Scene will run is shown before it runs, work that changes the
system is confirmed first and escalates through the desktop's own
authorisation agent, and the launcher reports what it actually observed: an
unavailable tool, a permission failure, a timeout, a cancellation, or a
non-zero exit.

%prep
%autosetup -n %{name}-%{version} -p1 -a1
%cargo_prep -v vendor

%build
%cargo_build
%{cargo_license_summary}
%{cargo_license} > LICENSE.dependencies
%{cargo_vendor_manifest}

%install
install -Dpm0755 target/release/%{name} %{buildroot}%{_bindir}/%{name}
install -Dpm0644 data/%{appid}.desktop %{buildroot}%{_datadir}/applications/%{appid}.desktop
install -Dpm0644 data/%{appid}.metainfo.xml %{buildroot}%{_metainfodir}/%{appid}.metainfo.xml
install -Dpm0644 data/%{appid}.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/%{appid}.svg
install -Dpm0644 data/%{name}.1 %{buildroot}%{_mandir}/man1/%{name}.1

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/%{appid}.desktop
appstream-util validate-relax --nonet %{buildroot}%{_metainfodir}/%{appid}.metainfo.xml
# Scene's own suite. The GTK smoke test prints its reason and passes where
# there is no display, which is every build environment.
%cargo_test

%files
%license LICENSE
%license LICENSE.dependencies
%license cargo-vendor.txt
%doc README.md
%{_bindir}/%{name}
%{_datadir}/applications/%{appid}.desktop
%{_metainfodir}/%{appid}.metainfo.xml
%{_datadir}/icons/hicolor/scalable/apps/%{appid}.svg
%{_mandir}/man1/%{name}.1*

%changelog
* Mon Aug 24 2026 Scene contributors <scene@example.invalid> - 0.1.0-1
- Milestone 8: first packaged build.
