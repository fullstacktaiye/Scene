# Scene 0.1.0 — release notes

The first packaged release. It proves the launcher core: activation, search
and ranking, typed and bounded actions, distro capability adapters, KDE-first
integration, and now packages for Fedora, Debian and Arch.

It is a 0.1: read the limits below before installing it as your only launcher.

## What is in it

- A single-instance GTK 4 launcher activated by a global shortcut, with the
  keyboard contract in `scene(1)`.
- Ranked results from installed applications, folders, links, KDE Places,
  recent documents, browser bookmarks, open windows, Activities, System
  Settings modules, power and session actions, processes, global shortcuts,
  Baloo file search, and Scene's own commands.
- Answers that exist only while a query asks for them: arithmetic and unit
  conversion, currency, timezones, colours, characters, web shortcuts, package
  queries, and an explicit `run PROGRAM ARGUMENT…` form.
- Typed actions with visible commands, confirmation before anything that
  changes the system, and reported outcomes that distinguish an unavailable
  tool, a permission failure, a timeout, a cancellation and a non-zero exit.
- Scene Settings (`Ctrl+,`): shortcut and Copilot-key status, provider order
  and enable/disable, history switches, and user-level autostart.
- Packages for the three supported families — an RPM, a `.deb` and an Arch
  package, each built offline from a reproducible release tarball, each running
  the test suite during its build, and each shipping a debug package.

## Installing

Build a package with `./scripts/package.sh` — see
[`packaging.md`](packaging.md) — and install the result:

```sh
sudo dnf install ./target/packages/fedora/scene-0.1.0-1.fc44.x86_64.rpm
sudo apt install ./target/packages/debian/scene_0.1.0-1_amd64.deb
sudo pacman -U ./target/packages/arch/scene-0.1.0-1-x86_64.pkg.tar.zst
```

For a development install that the desktop shortcut runs,
`./scripts/install-user.sh`.

After installing, bind a shortcut in KDE's own settings — Scene reports the
binding KDE recorded but never writes it. `Meta+Space` is the fallback the
desktop entry proposes.

## Verified on

| What | Where |
| --- | --- |
| The launcher itself | Fedora 44, KDE Plasma 6.7, Wayland, GTK 4.22 |
| Global activation, toggling, settings, autostart | The same machine, by hand |
| Package adapter command vectors | Fedora 44 host, `debian:stable-slim`, `archlinux:latest` |
| The three distro packages | `fedora:44`, `debian:unstable`, `archlinux:latest` containers; each ran the 90-test suite during its own build |
| Startup, indexing, latency, memory | Fedora 44, this machine — [`measurements.md`](measurements.md) |

## Known limits

### Desktop and session

- **A Wayland client cannot place its own window.** Centring is a KWin setting
  (System Settings → Window Management → Window Behavior → Advanced → Window
  placement: *Centered*), not something Scene can assert. Scene reports the
  limit rather than simulating placement.
- **Starting `scene` from a shell opens it behind other windows.** A shell
  gives no activation token. The global shortcut is the path that works, and
  the path this release is designed around.
- **The Copilot key is best effort.** It is a `Shift+Super+F23` chord emitting
  `XF86Assistant`, which KDE's Qt-based recorder cannot bind. Scene detects and
  reports what it observes and never installs or removes the XKB workaround.
  See [`copilot-key.md`](copilot-key.md).
- **KDE Plasma is the only complete desktop target.** Other sessions get the
  desktop-entry launch path, a plain unsupported-session report, and no KDE
  shortcut integration.
- **KDE-specific providers need KDE.** Windows, Activities, Places, System
  Settings modules and Baloo file search go through KDE's own D-Bus services.
  Where those are absent each becomes one local unavailable result; the
  launcher and everything else keep working.
- GTK 4.12 or newer is required. Verified against 4.22 on Fedora and Arch, and
  4.22 in the Debian unstable build container; trixie's 4.18 also satisfies it.
- **The .deb is a Debian unstable package, and Debian 13 cannot build Scene
  with its own toolchain.** Scene needs rustc 1.92 — the gtk4-rs 0.11 crate
  tree's requirement, not Scene's own code — while trixie ships 1.85 and
  trixie-backports has nothing newer. So the package is built in unstable
  (rustc 1.95), which also gives it `Depends: libc6 (>= 2.43)`; trixie has
  2.41, so it will not install there. On trixie, build Scene with a rustup
  toolchain instead.

### What Scene reports, and what it cannot

- **`Started` is not `Succeeded`.** An application handed to the desktop has no
  exit status Scene can read, so Scene says it started rather than claiming it
  worked. A program Scene spawns itself is watched for 400 ms: one that dies in
  that window reports its real exit status, and if the launcher has already
  closed it comes back to show the failure. A program that fails *after* that
  window is not observed, and Scene does not pretend otherwise.
- **Escape cannot cancel a launch.** Cancellation applies to bounded read-only
  work. A graphical program Scene has already started is not killed by closing
  the launcher.

### Capabilities and providers

- **Mutations go through `pkexec` only.** `install` and `remove` escalate
  through the desktop's authorisation agent. There is no `sudo` fallback and no
  password prompt of Scene's own; without `pkexec` they report as unavailable.
- **No real install or removal has been performed from Scene on a development
  machine.** The elevated path is covered by unit tests over the plan, policy
  and confirmation text, and by running the install argument vectors against a
  deliberately nonexistent package in containers.
- **Firefox bookmarks are read from the last checkpoint.** Firefox holds a
  write lock on `places.sqlite` for as long as it runs, so Scene reads the file
  immutably, without taking a lock. That ignores the write-ahead log: a
  bookmark added moments ago can be missing until Firefox writes it back. The
  alternative measured five seconds of every start and returned nothing.
- **Scene does not build a file index.** File search queries Baloo's existing
  index; content search is off by default and opt-in in settings. If Baloo is
  disabled or empty, Scene says so instead of scanning the disk.
- **Currency conversion needs the network once.** Rates are the European
  Central Bank's daily reference set, fetched over HTTPS and cached under
  `$XDG_CACHE_HOME/scene`. Offline and uncached, the conversion reports as
  unavailable.
- **Command history is off by default**, as is Baloo content search. Both are
  separately switchable and separately clearable.
- **Recent/frequent ranking is bounded on purpose.** Use can lift a result
  within its group and can never overturn a title match or cross a group
  boundary. `SCENE_HISTORY=off`, or the settings switch, turns it off.
- **Some capabilities are declined rather than missing.** Browser tabs and
  history, dictionary and spellcheck, KDE help search, Plasma internals, and
  application-specific session runners each produce a result explaining the
  decision when searched for. See [`krunner-migration.md`](krunner-migration.md).
- **`run` is not a shell.** It parses an argument vector, shows it, and asks
  before running it. Pipelines, redirection, globbing, substitutions and
  aliases are not expanded — deliberately, because search text never becomes a
  command.

### Configuration

- The configuration file is versioned. This release writes format 2; a format 1
  file from an earlier Scene is upgraded at startup and the previous file is
  kept beside it as `config.ini.format-1`. A file from a *newer* Scene is read
  for what this one recognises and copied aside before it is ever replaced.
- The ranking history in `$XDG_STATE_HOME/scene/history` is versioned too, but
  it is not migrated: a file whose first line names a format this Scene does
  not know starts empty and fills again with use. It is a ranking hint rather
  than something the user wrote, and rebuilding it costs nothing but a few
  days of ordinary use.

### Not yet done

- **The one-week KRunner-unbound field trial is not complete.** Milestone 6's
  implementation is finished, but its acceptance gate is daily-driver evidence,
  and that has not been recorded. Parity gaps may exist that no test found; see
  [`krunner-parity.md`](krunner-parity.md) for the track and
  [`krunner-migration.md`](krunner-migration.md) for the trial procedure.
- **The launcher has not been run on a Debian or Arch desktop.** Their packages
  build and their tests pass in containers, and the adapter command vectors are
  verified, but no one has activated the window there.
- **Accessibility checks are unfinished.** Accessible names and roles for newer
  result types, a selected state that does not rely on hue, and reduced-motion
  and high-contrast behaviour for the newer surfaces are unverified
  (`krunner-parity.md` §3.4).
- **Nothing is published.** There is no release tag, so `Source0`'s URL does not
  resolve yet; the AppStream metainfo carries no screenshots; and the Arch
  `PKGBUILD` still ships `SKIP` checksums. [`packaging.md`](packaging.md) lists
  what publishing needs.
