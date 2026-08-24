# Scene against the Fedora packaging guidelines

`PRODUCT_PLAN.md` Milestone 2 asked for Scene's desktop entry and build to be
checked against the [Fedora packaging
guidelines](https://docs.fedoraproject.org/en-US/packaging-guidelines/). That
check was never performed; Milestone 4.5 performs it and records the result
here. **Building the package itself is Milestone 8** — this document is the
list of requirements that work will have to satisfy, and what already
satisfies them.

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
(verified). Scene installs the file unmodified, so the spec may use
`desktop-file-validate` in `%check` rather than `desktop-file-install`; the
`BuildRequires` is Milestone 8's to add.

The entry declares `Name`, `GenericName`, `Categories=Utility;` and
`StartupNotify=false`, which are the fields the guidelines single out. The
`StartupNotify=false` is deliberate and correct: Scene is activated by a
shortcut and re-presents an existing window, so a startup-notification cursor
would be wrong.

### Icon — not met

> The short name without file extension is preferred, because it allows for
> icon theming.

The entry uses `Icon=system-search`, a short name — but it is *another
package's* icon, from the active icon theme, not one Scene ships. A Fedora
package for a GUI application is expected to install its own icon under
`%{_datadir}/icons/hicolor/`, and relying on a theme's stock name means Scene's
launcher entry looks like whatever the theme happens to have.

**Missing:** `dev.scene.Scene.svg` (scalable) under
`%{_datadir}/icons/hicolor/scalable/apps/`, and `Icon=dev.scene.Scene` in the
entry. Scene has no icon of its own yet, so this is design work, not packaging
work.

### AppStream metainfo — met

> If a package contains a GUI application, then it SHOULD install a
> `.metainfo.xml` file into `%{_metainfodir}`. […] The AppData files MUST
> correctly validate using `appstream-util validate-relax`. […] Application's
> AppData file MUST be named with the same root as the .desktop file.

Added by Milestone 4.5: `data/dev.scene.Scene.metainfo.xml`, sharing the root
of `dev.scene.Scene.desktop` as required.

`appstream-util` (from `libappstream-glib`) is not installed on this machine,
so validation was run with `appstreamcli validate --no-net`, which is the same
check from the AppStream project itself. It passes, with one *pedantic* note:
`cid-contains-uppercase-letter`. That is inherent to the reverse-DNS id
`dev.scene.Scene`, which must keep matching the desktop entry name and the
GTK application id in `src/main.rs`; well-known applications such as
`org.gnome.TextEditor` carry the same note. Milestone 8 should still run
`appstream-util validate-relax --nonet` in `%check` with
`BuildRequires: libappstream-glib`, because that is the tool the guideline
names.

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
the spec as a comment plus a combined SPDX expression.

### Rust build — the dependency tree exists, the spec does not

The Rust guidelines apply to Scene as a **non-crate Rust project**: an
application, not published on crates.io.

| Requirement | State |
| --- | --- |
| `BuildRequires: cargo-rpm-macros` | Available in Fedora 44 (`cargo-rpm-macros-28.5-1.fc44`, verified) |
| `%cargo_generate_buildrequires` in `%generate_buildrequires` | Not written yet — Milestone 8 |
| Dependencies packaged, so generated BuildRequires resolve | Verified: `rust-gtk4-devel-0.11.4-1.fc44` and `rust-gtk4+v4_12-devel-0.11.4-1.fc44` are in Fedora 44, which is exactly `gtk = { package = "gtk4", version = "0.11", features = ["v4_12"] }` |
| MUST NOT ship crate sources or `crate(...)` provides | Nothing to do: Scene has no `-devel` subpackage |
| MUST NOT use a `rust-` source package name | The package is `scene` |
| `%cargo_install` SHOULD NOT be used; copy from `target/rpm/*` instead | Milestone 8 |
| `$RUSTFLAGS` from `%build_rustflags` | Handled by `%cargo_build`; Milestone 8 |

Scene's single direct dependency is the reason this is short. A crate that
Fedora does not package would mean either packaging it or a bundling exception.

### Package independence — met

> Packages that contain a visible `.desktop` file SHOULD NOT have a `Requires`,
> `Recommends`, or `Supplements` on any other package containing a visible
> desktop file.

Scene requires GTK 4 and an icon theme, neither of which ships a visible
desktop entry. The optional integrations are detected on `PATH` at runtime and
must stay `Recommends` at most — Scene deliberately keeps working when
`konsole`, `dnf`, `pkexec` or `xdg-open` are absent, so none of them may be a
hard `Requires`.

## Summary of what is still missing

| Gap | Owner |
| --- | --- |
| An icon of Scene's own, installed under `hicolor` | Design, then Milestone 8 |
| `<screenshots>` in the metainfo | Milestone 8 (needs a published URL) |
| The spec file itself: `%generate_buildrequires`, `%cargo_build`, `%cargo_license_summary`, `%check` with `desktop-file-validate` and `appstream-util validate-relax` | Milestone 8 |
| A versioned release tarball with a `SourceURL`-compliant `Source0` | Milestone 8 |
| The real copyright holder in `LICENSE` | Whenever the repository owner confirms it |
| A build in mock and an `rpmlint` run | Milestone 8 |

Nothing found here blocks Milestone 4.5 or changes the launcher's behaviour.
Two of the gaps were closed while writing this — the license text and the
metainfo file — because both were missing outright rather than deferred.
