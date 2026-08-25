# Scene against the Fedora packaging guidelines

`PRODUCT_PLAN.md` Milestone 2 asked for Scene's desktop entry and build to be
checked against the [Fedora packaging
guidelines](https://docs.fedoraproject.org/en-US/packaging-guidelines/). That
check was never performed; Milestone 4.5 performed it and recorded the result
here. **Milestone 8 then built the package**: the spec is
`packaging/fedora/scene.spec` and the procedure is
[`packaging.md`](packaging.md). This document stays the requirement-by-
requirement record, updated with what the real build turned out to need.

`desktop-file-validate` passing is a much weaker statement than this. It checks
one file against the desktop-entry spec. The guidelines below are about what a
Fedora package must contain and prove.

## How this was checked

The guidelines' own source was read rather than the rendered site, which is
behind a bot challenge: `git clone https://pagure.io/packaging-committee.git`,
commit `1fb4fbd` (2026-04-17), the last commit before the repository was
migrated to Fedora Forge. The pages used are `index.adoc` (§ Desktop Files),
`AppData.adoc`, `Rust.adoc`, and `LicensingGuidelines.adoc`.

Everything marked *verified* below was run on the development machine —
Fedora 44, `rpm 6.0.2`, `desktop-file-utils`, `appstreamcli 1.1.3`.

## What the guidelines require

### Desktop entry — met

> It is not simply enough to just include the .desktop file in the package, one
> MUST run `desktop-file-install` (in `%install`) OR `desktop-file-validate`
> (in `%check` or `%install`) and have `BuildRequires: desktop-file-utils`.

`data/dev.scene.Scene.desktop` passes `desktop-file-validate` with no output
(verified). Scene installs the file unmodified, so the spec uses
`desktop-file-validate` in `%check` rather than `desktop-file-install`, with
`BuildRequires: desktop-file-utils` (added in Milestone 8, and run in every
package build since).

The entry declares `Name`, `GenericName`, `Categories=Utility;` and
`StartupNotify=false`, which are the fields the guidelines single out. The
`StartupNotify=false` is deliberate and correct: Scene is activated by a
shortcut and re-presents an existing window, so a startup-notification cursor
would be wrong.

### Icon — met, in Milestone 8

> The short name without file extension is preferred, because it allows for
> icon theming.

The entry used to say `Icon=system-search`, a short name — but it was *another
package's* icon, from the active icon theme, not one Scene ships. A Fedora
package for a GUI application is expected to install its own icon under
`%{_datadir}/icons/hicolor/`, and relying on a theme's stock name meant Scene's
launcher entry looked like whatever the theme happened to have.

**Closed in Milestone 8.** `data/dev.scene.Scene.svg` is Scene's own icon — the
launcher surface itself, in the palette `src/style.css` uses — installed to
`%{_datadir}/icons/hicolor/scalable/apps/dev.scene.Scene.svg`, with
`Icon=dev.scene.Scene` in the entry and `Requires: hicolor-icon-theme` in the
spec. `scripts/install-user.sh` installs it under `~/.local/share/icons` so a
development install looks the same.

### AppStream metainfo — met

> If a package contains a GUI application, then it SHOULD install a
> `.metainfo.xml` file into `%{_metainfodir}`. […] The AppData files MUST
> correctly validate using `appstream-util validate-relax`. […] Application's
> AppData file MUST be named with the same root as the .desktop file.

Added by Milestone 4.5: `data/dev.scene.Scene.metainfo.xml`, sharing the root
of `dev.scene.Scene.desktop` as required.

`appstream-util` (from `libappstream-glib`) is not installed on this machine,
so validation here was run with `appstreamcli validate --no-net`, which is the
same check from the AppStream project itself. The package build runs the tool
the guideline names — `appstream-util validate-relax --nonet` in `%check`, with
`BuildRequires: libappstream-glib` — and it passes there too. It passes with
one *pedantic* note:
`cid-contains-uppercase-letter`. That is inherent to the reverse-DNS id
`dev.scene.Scene`, which must keep matching the desktop entry name and the
GTK application id in `src/main.rs`; well-known applications such as
`org.gnome.TextEditor` carry the same note.

**Not yet present:** a `<screenshots>` block. AppStream wants at least one
screenshot with a caption for an application, and it needs a stable public URL,
which means Milestone 8 or a published release.

### License — met, with one thing to confirm

> If the source package includes the text of the license(s) in its own file,
> then that file […] must be included in the `%files` list flagged with the
> `%license` directive.

`Cargo.toml` declared `license = "MIT"` with no license file in the repository
at all, so there was nothing for `%license` to point at. Milestone 4.5 adds
`LICENSE` with the MIT text.

Its copyright line names the copyright holder, Taiye Babatope, so `%license`
has something to point at and `rpm -q --licensefiles` will answer.

Note the Rust-specific rule that follows from static linking:

> Rust executables (and shared libraries) contain code that originates in other
> packages […] This needs to be taken into account by maintaining a separate
> `License` tag for the subpackage that contains these binaries.

Scene's binary statically links the whole `gtk4` crate tree, so the spec's
`License:` tag is *not* `MIT`. It is the conjunction of MIT with every
dependency's license, generated with `%cargo_license_summary` and pasted into
the spec as a comment plus a combined SPDX expression. Verified in the
Milestone 8 build: the summary is thirteen distinct licence expressions, the
tag is their conjunction, and `%{cargo_license}` writes the per-crate breakdown
to `LICENSE.dependencies`, which is installed with `%license` alongside
`cargo-vendor.txt` from `%{cargo_vendor_manifest}`.

### Rust build — verified against the real build

The Rust guidelines apply to Scene as a **non-crate Rust project**: an
application, not published on crates.io.

| Requirement | State |
| --- | --- |
| `BuildRequires: cargo-rpm-macros` | Verified: `cargo-rpm-macros-28.5-1.fc44` in Fedora 44, and what the spec requires |
| `%cargo_generate_buildrequires` in `%generate_buildrequires` | **Deliberately not used.** That macro emits a `crate(…)` BuildRequires per dependency, which is the unbundled path. Scene builds from a vendor tarball instead — see below |
| MUST NOT ship crate sources or `crate(...)` provides | Nothing to do: Scene has no `-devel` subpackage |
| MUST NOT use a `rust-` source package name | The package is `scene` |
| `%cargo_install` SHOULD NOT be used; copy from `target/rpm/*` instead | Met: `%install` copies `target/release/scene`, which `%cargo_prep` symlinks to `target/rpm` |
| `$RUSTFLAGS` from `%build_rustflags` | Met by `%cargo_build`; the build log shows `-Copt-level=3 -Cdebuginfo=2 -Ccodegen-units=1 -Cstrip=none -Cforce-frame-pointers=yes` and Fedora's package notes linker spec |

**Bundled, not unbundled, and this is the honest statement of it.** The
Milestone 4.5 version of this table assumed Scene's one direct dependency would
resolve to packaged crates. Since Milestone 6, `Cargo.lock` resolves 131
crates, and requiring every one of them to exist in Fedora is not realistic for
this project. The spec therefore takes the vendored path: `Source1` is the
`cargo vendor` tarball, `%prep` runs `%cargo_prep -v vendor`, and the build is
offline (`%cargo_prep` writes `[net] offline = true` itself, verified in the
macro source).

Two consequences follow, and both are handled rather than hidden. The `License`
tag carries every bundled crate's licence, as the Licensing guidelines require
for statically linked Rust code. And submitting this package to Fedora proper
would need either those crates packaged or a bundling exception — this spec is
built and installed locally, not submitted, which `packaging.md` says plainly.

### Package independence — met

> Packages that contain a visible `.desktop` file SHOULD NOT have a `Requires`,
> `Recommends`, or `Supplements` on any other package containing a visible
> desktop file.

Scene requires GTK 4 and an icon theme, neither of which ships a visible
desktop entry. The optional integrations are detected on `PATH` at runtime and
must stay `Recommends` at most — Scene deliberately keeps working when
`konsole`, `dnf`, `pkexec` or `xdg-open` are absent, so none of them may be a
hard `Requires`.

## What Milestone 8 closed, and what is left

| Gap | State |
| --- | --- |
| An icon of Scene's own, installed under `hicolor` | Closed: `data/dev.scene.Scene.svg` |
| The spec file: `%cargo_build`, `%cargo_license_summary`, `%check` with `desktop-file-validate` and `appstream-util validate-relax` | Closed: `packaging/fedora/scene.spec`, built and linted |
| A versioned release tarball with a `SourceURL`-compliant `Source0` | Closed: `scripts/release-tarball.sh`, and `Source0` is the canonical GitHub archive URL. Nothing is published at that URL yet, because there is no release tag |
| An `rpmlint` run | Closed: run on the spec, the source RPM and all three binary RPMs in every build, and its report is exported beside the packages |
| A build in mock | **Not run.** mock needs privileges a container build does not have. The equivalent property — that the declared BuildRequires are complete — is proved instead by building the SRPM first and then installing *only* `dnf builddep`'s answer in a fresh `fedora:44` container. A mock build remains the thing to run before any submission |
| `<screenshots>` in the metainfo | **Still open.** Needs a published image URL |
| The real copyright holder in `LICENSE` | Whenever the repository owner confirms it |

The last rpmlint run reports two errors and one warning, and neither is a
packaging defect: `spelling-error ('authorisation')`, which is British English
checked against `en_US`, and `invalid-url Source1`, which every vendored Rust
package carries because a vendor tarball is generated rather than fetched. The
`no-manual-page-for-binary` warning that the first build reported was fixed
rather than filtered — `data/scene.1` is now written and installed.
