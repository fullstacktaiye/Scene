# Building and packaging Scene

Milestone 8 asks for reproducible development and packaging instructions, and
for packages on all three supported families. This is that procedure, together
with what it was verified to produce on this machine.

`docs/fedora-packaging.md` is the separate record of what Fedora's packaging
guidelines require and how Scene answers each requirement. This document is how
to run the builds.

## Development

Scene is one Rust binary against native GTK 4. The development headers are the
only system prerequisite:

```sh
sudo dnf install gtk4-devel sqlite-devel     # Fedora
sudo apt install libgtk-4-dev libsqlite3-dev # Debian / Ubuntu
sudo pacman -S gtk4 sqlite                   # Arch
```

Rust 1.92 or newer is required, which is older than Fedora's and Arch's current
toolchains and newer than Debian 13's — see [the Debian toolchain
note](#the-debian-toolchain-and-why-the-container-is-unstable).

```sh
cargo run --release        # build and launch
cargo test                 # 90 tests: 89 hermetic, 1 UI smoke
cargo fmt --check
cargo clippy --all-targets
./scripts/install-user.sh  # make the desktop shortcut run this build
scene --measure            # this machine's startup, indexing, latency, memory
```

`cargo build` alone does not change what the desktop shortcut starts; see the
comment at the top of `scripts/install-user.sh` for why, and run that script
instead.

## What a release is made of

```sh
./scripts/release-tarball.sh
```

writes two tarballs into `target/dist`:

| File | What it is |
| --- | --- |
| `scene-VERSION.tar.gz` | The sources: every file git tracks or would track, plus a generated `LICENSE.dependencies` |
| `scene-VERSION-vendor.tar.gz` | Every crate `Cargo.lock` resolved, as `cargo vendor` produced them |

The vendor tarball exists because **all three package builds run offline**. A
packaging build that downloads from crates.io is not reproducible, and Fedora's
guidelines do not allow it, so the dependency set is resolved once — with
`--locked`, so it is what `Cargo.lock` says and not what crates.io offers today.

Both tarballs are byte-for-byte reproducible from the same tree: the file list
is sorted, ownership is numeric root, every mtime is `SOURCE_DATE_EPOCH` (the
last commit's timestamp by default), and gzip is told not to record its own.
Running the script twice on an unchanged tree gives identical checksums, which
it prints for a release's checksum list. Checked rather than asserted: two
consecutive runs on 2026-08-24 produced the same pair of digests, and the
vendor tarball's digest was unchanged across every packaging run that day.

The source tarball is built from what the working tree contains rather than
from `HEAD`, so packaging a change that has not been committed yet does not
silently package the previous version of it.

## Building the packages

```sh
./scripts/package.sh              # fedora, debian and arch
./scripts/package.sh fedora       # one of them
```

Each target builds in a container rather than on this machine, for two reasons:
the package is built against the distribution it targets, and **each build
installs its dependencies from the packaging metadata itself** — `dnf builddep`
from the source RPM, `mk-build-deps` from `debian/control`, `makepkg
--syncdeps` from the `PKGBUILD`. An incomplete dependency list fails the build
instead of quietly borrowing something the image already carried, which is the
property a mock or pbuilder run is really there to prove.

Everything a build needs is assembled into `target/dist/context-TARGET`, so the
container never sees the working tree. Results land in `target/packages/TARGET`,
each with its linter's report beside it:

| Target | Image | Produces | Linted by |
| --- | --- | --- | --- |
| `fedora` | `fedora:44` | `scene-VERSION-1.fc44.x86_64.rpm`, its `-debuginfo` and `-debugsource`, and the source RPM | `rpmlint` |
| `debian` | `debian:unstable` | `scene_VERSION-1_amd64.deb` | `lintian` |
| `arch` | `archlinux:latest` | `scene-VERSION-1-x86_64.pkg.tar.zst` | `namcap` |

The linter reports are exported rather than swallowed: a build that produced a
package with problems cannot be mistaken for a clean one.

Every one of the three runs Scene's own test suite as part of the package
build — `%check` for RPM, `override_dh_auto_test` for the .deb, `check()` for
the Arch package. The GTK smoke test prints its reason and passes where there
is no display, which is every build container.

### The Debian toolchain, and why the container is unstable

Scene needs **rustc 1.92 or newer** — `Cargo.toml` says so, and the requirement
comes from the gtk4-rs 0.11 crate tree, not from Scene's own code. Verified on
2026-08-24:

| Where | rustc |
| --- | --- |
| Fedora 44 | 1.97.1 |
| Arch, current | 1.98.0 |
| Debian 13, trixie | 1.85.0 — **too old** |
| Debian 13, trixie-backports | 1.85.0; backports has nothing newer |
| Debian unstable, forky/sid | 1.95.0 |

So the .deb is built in `debian:unstable`, and `debian/control` declares
`rustc (>= 1.92)`. Debian 13 cannot build Scene with its own toolchain: on
trixie the build needs a rustup toolchain, and the resulting package is not one
trixie could rebuild from source. That is a fact about the dependency tree
rather than a packaging choice, and it will resolve itself when Debian ships a
newer rustc.

**The .deb this produces is a Debian unstable package, not a trixie one.**
Building against unstable's glibc gives it `Depends: libc6 (>= 2.43)`, and
trixie has 2.41, so it will not install there — a fact worth stating flatly
rather than discovering at `apt install` time. A trixie user needs a rustup
toolchain and a build on trixie itself; the GTK requirement is not the
obstacle, since trixie's 4.18 is newer than the 4.12 Scene asks for.

## What each package installs

The same layout on all three:

```text
/usr/bin/scene
/usr/share/applications/dev.scene.Scene.desktop
/usr/share/metainfo/dev.scene.Scene.metainfo.xml
/usr/share/icons/hicolor/scalable/apps/dev.scene.Scene.svg
/usr/share/man/man1/scene.1
```

plus each distribution's own place for `LICENSE` and the generated
`LICENSE.dependencies`: `%license` for RPM,
`/usr/share/doc/scene/` for the .deb, `/usr/share/licenses/scene/` for Arch.

Scene links its dependencies statically, so every package states what is inside
it. `LICENSE.dependencies` lists all 131 crates with their versions and
licences, generated from `Cargo.lock` by `scripts/dependency-licenses.sh`; the
RPM regenerates the same thing with Fedora's `%{cargo_license}` and adds
`cargo-vendor.txt` from `%{cargo_vendor_manifest}`.

Nothing but `hicolor-icon-theme` is a runtime dependency beyond the shared
libraries the binary links. That is deliberate: Scene detects the tools it can
use — a terminal, `dnf`/`apt`/`pacman`, `pkexec`, KDE's shortcut editor, Baloo
— and reports the missing ones as unavailable capabilities rather than refusing
to start.

## Building without containers

The container is a convenience, not a requirement. On the target distribution:

```sh
# Fedora
./scripts/release-tarball.sh
rpmdev-setuptree
cp target/dist/*.tar.gz ~/rpmbuild/SOURCES/
cp packaging/fedora/scene.spec ~/rpmbuild/SPECS/
rpmbuild -ba ~/rpmbuild/SPECS/scene.spec

# Debian
./scripts/release-tarball.sh
tar xf target/dist/scene-0.1.0.tar.gz -C /tmp
tar xf target/dist/scene-0.1.0-vendor.tar.gz -C /tmp/scene-0.1.0
cp -r packaging/debian /tmp/scene-0.1.0/debian
cd /tmp/scene-0.1.0 && dpkg-buildpackage -us -uc -b

# Arch
./scripts/release-tarball.sh
cp packaging/arch/PKGBUILD target/dist/ && cd target/dist && makepkg -s
```

A mock build is the Fedora equivalent of the container path
(`mock -r fedora-44-x86_64 --rebuild target/packages/fedora/*.src.rpm`) and
needs mock installed and the user in the `mock` group; the container build
proves the same BuildRequires completeness without that privilege.

## Publishing a release

Not done yet, and the three things it needs are known:

1. Tag `v0.1.0` and publish the source tarball, so `Source0`'s URL
   (`%{url}/archive/v%{version}/%{name}-%{version}.tar.gz`) resolves. The spec
   already uses the canonical GitHub form; nothing is published at it yet.
2. Add `<screenshots>` to `data/dev.scene.Scene.metainfo.xml`, which needs a
   published image URL. AppStream validation passes without them, and software
   centres show the entry without a picture.
3. Replace the `SKIP` entries in `packaging/arch/PKGBUILD` with the checksums
   `scripts/release-tarball.sh` prints for the published tarballs.

## Verified

Run on 2026-08-24, Fedora 44 host, Docker 29.7.2. Every one of the three built,
ran Scene's 90 tests during its own build, and produced a package with the same
seven installed paths.

| Target | Produced |
| --- | --- |
| `fedora:44` | `scene-0.1.0-1.fc44.x86_64.rpm` (2.2 MB), `-debuginfo` (13 MB), `-debugsource` (3.2 MB), `scene-0.1.0-1.fc44.src.rpm` (29 MB) |
| `debian:unstable` | `scene_0.1.0-1_amd64.deb` (2.0 MB), `scene-dbgsym_0.1.0-1_amd64.deb` (19 MB) |
| `archlinux:latest` | `scene-0.1.0-1-x86_64.pkg.tar.zst` (2.7 MB), `scene-debug-…` (27 MB) |

Each declares only what it actually links: GTK 4 (`libgtk-4-1 (>= 4.12.0)` on
Debian, `gtk4` on Arch), SQLite, the usual C libraries, and
`hicolor-icon-theme` for the icon. The RPM's `%check` runs
`desktop-file-validate` and `appstream-util validate-relax --nonet` as well as
the test suite.

The Fedora RPM was installed into a scratch root and run on this machine: it
presented its window and printed its own measurements, so the packaged binary
is known to work rather than only to build.

### Linter findings, and what was done about them

| Finding | Answer |
| --- | --- |
| `rpmlint`: `no-manual-page-for-binary scene` | **Fixed**, not filtered: `data/scene.1` is written and installed by all three packages. |
| `lintian`: `debug-file-with-no-debug-symbols` | **Fixed**: the release profile carries no debug info, and `RUSTFLAGS` cannot add it because cargo passes the profile's own `-Cdebuginfo` afterwards. `CARGO_PROFILE_RELEASE_DEBUG=2` sets it where it wins, so the `-dbgsym` package now carries real symbols. The Arch `PKGBUILD` does the same, and its debug package went from an empty shell to 27 MB. |
| `namcap`: undefined `ring_core_*` symbols at link time | **Fixed**: makepkg's global LTO put `-flto=auto` into `CFLAGS`, so the C and assembly `ring`'s build script compiles became bitcode the ordinary Rust link step could not resolve. `options=('!lto')` — cargo decides Scene's optimisation, makepkg does not need to. |
| `rpmlint`: `spelling-error ('authorisation')` | Not a defect. The repository is written in British English; `rpmlint` checks against `en_US`. |
| `rpmlint`: `invalid-url Source1: scene-0.1.0-vendor.tar.gz` | Expected. A vendor tarball is generated at release time, not fetched from a URL; every vendored Rust package in Fedora carries this. |
| `lintian`: `initial-upload-closes-no-bugs` | A Debian archive convention — a first upload should close its ITP bug. This package is not being uploaded to Debian. |
| `namcap`: `Dependency … detected and implicitly satisfied` (glibc, cairo, glib2, graphene, pango, libgcc) | Informational. Each is pulled in by `gtk4`; listing them again would pin versions Scene does not care about. |
| `namcap`: `scene-debug E: Symlink … points to non-existing ../../../../bin/scene` | A false positive on any split-debug package: the build-id symlink points into the *main* package, which namcap does not have in front of it. |

Nothing in either list is an unaddressed packaging defect.
