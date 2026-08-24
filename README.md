# Scene

A fast, keyboard-first Linux launcher. See [`PRODUCT_PLAN.md`](PRODUCT_PLAN.md)
for what Scene is meant to become.

This repository contains the code-complete portion of **Milestone 6 — KDE
replacement foundation**. Its final acceptance gate is the documented
one-week KRunner-unbound field trial.

## Build and run

Scene is one Rust binary against native GTK 4.

```sh
# Fedora
sudo dnf install gtk4-devel

# Debian / Ubuntu
sudo apt install libgtk-4-dev

# Arch
sudo pacman -S gtk4
```

```sh
cargo run --release
```

To run it from a desktop shortcut instead, install it for your user:

```sh
./scripts/install-user.sh
```

That installs the binary, generates `~/.local/share/applications/dev.scene.Scene.desktop`
from `data/dev.scene.Scene.desktop` with an absolute `Exec`, installs the
AppStream metainfo, validates both, and stops any resident instance. Run it
after every change you want the shortcut to pick up: `cargo build --release`
writes `target/release/scene`, which is *not* the binary a desktop entry runs,
and a resident single-instance process would keep serving the old code
regardless. The desktop entry proposes `Meta+Space` as the fallback shortcut.
KDE remains the source of truth for the active binding and conflict handling:
search for **Scene Settings** (or press `Ctrl+,`) to see what Scene observed
and open KDE's native recorder. Packaging proper is Milestone 8.

Scene registers as a single instance. Activating it while hidden presents a
focused launcher with a cleared query; activating it again hides the same
window. It never leaves a competing launcher process or window.

## Using it

| Input | Behaviour |
| --- | --- |
| Printable text | Filters and re-ranks the results |
| Up / Down | Moves the selection, wrapping at either end |
| Enter | Runs the selected result |
| Escape | Clears the query; cancels a running action; withdraws a confirmation; on an empty query, hides the launcher |
| Click | Selects and runs, through the same path as Enter |
| Ctrl+K | Opens the selected result's primary and secondary actions |
| Ctrl+, | Opens shortcut and Copilot-key settings |
| Ctrl+Q | Quits Scene |

Results you use often, and used recently, rise within their provider group.
Scene Settings can disable or clear that history, enable and reorder providers,
opt into separate command history or Baloo content search, and configure
user-level autostart.

Before you type anything, the applications group shows only its top five —
the ones you actually use, then alphabetical order to fill. There are 81
installed applications on the development machine, and listing all of them
buries every other group. The heading says `5 of 81` so the short list is
never mistaken for the whole set, and typing searches all of them. Every other
group is a small catalogue Scene itself owns, so it shows in full.

Useful explicit query forms include:

| Query | Result |
| --- | --- |
| `= EXPRESSION` / `convert VALUE UNIT to UNIT` | Calculate or convert locally |
| `convert 10 USD to EUR` | Convert with cached ECB reference rates |
| `time Europe/London` | Show a named timezone |
| `color VALUE` / `char U+1F680` | Convert a colour or find a character |
| `run PROGRAM ARGUMENT…` | Preview and confirm a bounded, shell-free command |
| `file TERM` | Search Baloo by filename (content search is opt-in) |
| `process TERM` | Find a current-user process and offer confirmed signals |
| `pkg NAME` | Show merged package metadata and installed/available state |
| `install NAME` | Install it, behind an explicit confirmation |
| `remove NAME` (or `uninstall NAME`) | Remove it, behind an explicit confirmation |

See [`docs/krunner-migration.md`](docs/krunner-migration.md) for the gains,
intentional omissions, shortcut hand-off, and field-trial procedure.

## Layout

```text
src/
├── main.rs          process lifecycle, activation, window reuse
├── apps.rs          installed application discovery
├── search.rs        matching, ranking, grouping, the built-in result set
├── integrations.rs  the provider registry and its contracts
├── packages.rs      distro capability adapters
├── platform.rs      KDE shortcut and Copilot capability detection
├── actions.rs       typed actions and their outcomes
├── system.rs        executable discovery and bounded subprocesses
├── ui.rs            window, search field, result list, footer, UI smoke suite
└── style.css        the launcher's appearance

scripts/
└── install-user.sh  install for the current user, for the desktop shortcut

data/
├── dev.scene.Scene.desktop       the desktop entry
└── dev.scene.Scene.metainfo.xml  AppStream metadata for packaging
```

`ui.rs` renders what `search` produced and reports what `actions` returned. It
never decides either, so the styling can change without touching the logic.

## Milestone 6 status

- [x] Provider-labelled groups with persistent enable/disable and priority
      controls.
- [x] Inline actions, including declared desktop actions, reachable with
      `Ctrl+K`.
- [x] Optional, separately clearable command history and an explicit `run`
      prefix with no shell expansion.
- [x] Calculator/unit, currency, timezone, colour, character, web-shortcut,
      window, Activity, Baloo, recent-file, KDE Places, bookmark, settings,
      power/session, process, shortcut, and merged package providers.
- [x] User-level autostart for a warm single-instance `--background` process.
- [ ] Complete and record the one-week KRunner-unbound field trial.

## Milestone 5 status

- [x] **KDE-first global activation.** The desktop entry declares
      `Meta+Space` as its fallback and the user install script deploys that
      exact entry. Repeated activation toggles one resident Scene window.
- [x] **Shortcut settings.** Scene reads the active bindings KDE recorded for
      its desktop action, shows the session and fallback separately, and opens
      `systemsettings kcm_keys` for recording and conflict handling. Scene
      never writes `kglobalshortcutsrc` itself.
- [x] **Observed Copilot-key status.** The settings test distinguishes an
      observed bindable `Meta+Shift+F23`, stock `XF86Assistant` that KDE/Qt
      cannot record, a bound desktop activation, and no observed event. It
      never treats the presence of an F23 capability bit as hardware proof.
- [x] **No hidden system changes.** The XKB workaround remains documented in
      `docs/copilot-key.md`; Scene does not install or remove it.

## Milestone 4.5 status

Four checklist items whose milestone had already passed were still open.
Milestone 4.5 collected and closed them, so that "Milestone 4 is done" cannot
quietly mean "done except for the parts nobody is tracking".

- [x] **Keyboard-only smoke path.** `src/ui.rs` now carries a UI smoke suite:
      one test that builds the real launcher against stated items, presents
      it, and drives the whole contract — activation and focus, typing,
      Up/Down with wrapping, the empty-result state, Enter, Escape clearing
      then closing, re-activation resetting query and selection, a
      confirmation that a result-row click cannot answer, and a long action
      cancelled from the keyboard. It skips itself with a printed reason where
      there is no display. Its stated limit: keys enter at `Launcher::key`,
      the method the window's key controller calls, so the compositor
      delivering a physical key press to that controller is what the manual
      pass covers.

      Driven by hand on 2026-08-24, Fedora 44 / KDE Plasma 6.7 / Wayland: the
      same sequence, from a real keyboard, including an `install` answer that
      stopped at its confirmation and was withdrawn with Escape. The run left
      its own evidence in `$XDG_STATE_HOME/scene/history` — `scene.reporting`,
      `packages.metadata` and `terminal.open`, and *not* the withdrawn
      `install`, because use is recorded when an action starts rather than
      when it is offered. On the next activation the terminal ranked above
      where it had sat before.
- [x] **Recent/frequent ranking.** `search::History` records what was chosen
      and lifts it within its group. The adjustment is bounded — 24 for
      frequency plus 15 for recency, under the 40-point gap the matcher puts
      between a title hit and a keyword hit — so use can reorder results
      within a group but can never overturn a title match or cross a group
      boundary. Ranking is still deterministic in the sense the plan means:
      the history is an argument to `search`, not ambient state, so the same
      query, items and history always give the same order. `SCENE_HISTORY=off`
      turns it off.
- [x] **Fedora packaging check.** Performed against the guidelines' own
      source and recorded in `docs/fedora-packaging.md`. The desktop entry and
      package independence already comply. Two gaps were closed rather than
      logged: the repository declared MIT with no `LICENSE` file for
      `%license` to point at, and shipped no AppStream metainfo, which a GUI
      application is expected to install. `data/dev.scene.Scene.metainfo.xml`
      passes `appstreamcli validate --no-net`. The spec file, an icon of
      Scene's own, screenshots and a mock build are Milestone 8's.
- [x] **Launched-program outcomes.** A detached program is now watched for
      400 ms: one that dies in that window reports its real exit status
      instead of the success Scene used to claim, and if the launcher has
      already closed it comes back to show the failure. An application handed
      to the desktop has no exit status Scene can read, so it reports
      `Started` — a different answer from `Succeeded` — and the built-in
      "What Scene Reports" result says so in words.

Declared `.desktop` actions such as “New Private Window” now use Milestone 6's
shared typed inline-action model.

## Milestone 4 status

Adapter commands verified against Fedora 44 (host), `debian:stable-slim`, and
`archlinux:latest`. The launcher itself is verified on Fedora 44 / KDE Plasma
6.7 / Wayland only.

- [x] Shared adapters for Debian/Ubuntu, Fedora, and Arch — one `Adapter`
      interface in `src/packages.rs` declaring six capabilities per family.
      Nothing in that module runs a process; it answers with a fully specified
      `Plan` that `actions` and `system` then execute under the existing
      bounded policy.
- [x] Package search, metadata, installed-package, and update capabilities.
      Each is a real result row whose subtitle is the argument vector it will
      run.
- [x] Mutation behind confirmation — `install` and `remove` are the first
      `ExecutionPolicy::Mutating` actions in Scene. They freeze their payload
      and need a deliberate Enter; a result-row click cannot confirm one.
- [x] Capability detection by executable, not distribution name. `/etc/os-release`
      only orders which adapter is probed first: a machine that calls itself
      Fedora but carries only `pacman` is detected as Arch, and one that names
      a family it has no tooling for is detected as nothing.
- [x] Missing tools, permission problems, and unsupported operations are
      reported in words. A capability that cannot run still produces a result
      saying why — a missing `rpm`, an absent `pkexec`, a package name that
      cannot be one.
- [x] Verified on all three families. Every adapter's argument vector was run
      in the environment it targets, for a package that exists, a package that
      does not, and the update query. The install vectors were checked against
      a deliberately nonexistent package, so they reach package resolution and
      change nothing.
- [x] Core launcher behaviour when no adapter is usable — running Scene with an
      empty `PATH` leaves the window, application discovery, folders, links,
      and Scene's own commands working; the terminal, system-information, and
      package providers each become one local unavailable result.

### What the live runs found

Three exit codes are answers rather than failures, and the adapters accept
them: `dpkg-query`, `rpm --query`, and `pacman --query --info` all exit 1 for a
package that is simply not installed; `pacman --sync --search` exits 1 when
nothing matches; `dnf check-update` exits 100 when updates exist. Everything
else non-zero is still reported as a failure with the tool's own message.
`dnf` is run with `--assumeno` so a repository-key prompt is declined rather
than waiting on closed input.

Re-run during Milestone 4.5 against `debian:stable-slim`, `archlinux:latest`
and `fedora:latest`, and every one of those exit codes still holds. The run
also confirmed the codes that are *not* accepted are failures worth reporting:
`apt-get install` answers 100 for a package that does not exist, and `dnf
install` and `pacman --sync` answer 1.

### Privilege

Mutation runs through `pkexec`, so the desktop's own authentication agent asks
for authorisation. Scene has no `sudo` fallback and no password prompt of its
own: when `pkexec` is absent, install and remove report that they are
unavailable and say why.

### Not verified

The GTK launcher itself has not been run on a Debian or Arch desktop — only its
adapter commands have. No real install or removal was performed on the
development machine; the elevated path is covered by unit tests over the plan,
the policy, and the confirmation text, and by running the install argument
vectors against a nonexistent package in containers.

## Milestone 3 status

Scene has a typed built-in integration registry. Adding an integration means
implementing its metadata and `search` contract in `src/integrations.rs`; the
GTK search UI remains unchanged. Providers can return an explicit error, which
becomes one local unavailable result rather than removing unrelated results.

- [x] Built-ins: installed applications, terminal launch, system information,
      configured directory, and detected package-manager information.
- [x] Registered read-only processes run off the GTK thread with a fixed
      timeout, null input, bounded stdout/stderr capture, and Escape-driven
      cancellation. Terminal launch is a separate detached policy.
- [x] Process outcomes distinguish unavailable tools, permission errors,
      cancellation, timeouts, non-zero exits, and captured output.
- [x] Mutating policy is structurally separate from read-only and detached
      work. It freezes the action payload and requires an explicit Enter
      confirmation; result-row clicks cannot accidentally confirm it.
- [x] `SCENE_DIRECTORY=/path/to/open scene` changes the directory integration.
      It defaults to the user's home directory when unset.

Automated verification covers the provider-error isolation, configuration,
confirmation decision, cancellation-before-spawn, bounded output, captured
stdout/stderr, and timeout behavior using temporary fake executables.

Milestone 4 replaced the placeholder `--version` package query with the real
capability adapters, and added a second provider contract — `answer`, for
results that only exist while a query asks for them. Adding either kind of
provider still needs no change to `ui.rs`.

## Milestone 2 status

Verified on Fedora 44, KDE Plasma 6.7, Wayland, GTK 4.22.

- [x] Discover Linux desktop entries — through GIO's application model rather
      than a hand-rolled parser, so `NoDisplay`, `Hidden`, `OnlyShowIn` and
      `NotShowIn` are honoured for free. 292 entries on this machine, 81 shown;
      the 211 hidden are almost all `NoDisplay=true`.
- [x] Asynchronous or incremental — measured at 12-19 ms warm, 27 ms cold, run
      at startup before any window exists. That is short enough that a thread
      would be machinery without a purpose; the figure is in `apps.rs` so a
      regression is visible. Re-indexing is driven by `AppInfoMonitor`, so an
      application installed while Scene is resident appears without a restart.
- [x] Render icons, names, descriptions and categories — confirmed against a
      live window, including applications whose icons are files on disk rather
      than theme names (Flatpak, Steam, AppImage).
- [x] Deterministic fuzzy search and ranking — titles outrank keywords, which
      outrank descriptions, and `AppInfo::all`'s unspecified order is replaced
      with a stable sort.
- [x] Present launch failures in the UI — `Action::Launch` reports through the
      same `Outcome` states as everything else.
- [x] A valid `.desktop` entry for Scene itself — `data/dev.scene.Scene.desktop`
      passes `desktop-file-validate`.
- [x] Recent/frequent ranking — added in Milestone 4.5, once the deterministic
      baseline the plan requires was in place.

Launching was verified end to end: executing a discovered item returned
`Succeeded("Opened KCalc")` and the process actually appeared. That check is
not in the test suite, because it depends on KCalc being installed and the
plan requires hermetic tests.

Applications launch through `AppInfo::launch` with the display's launch
context, so the new window receives the Wayland activation token and opens in
front. Note the corollary: starting `scene` itself from a shell gets *no*
token, so its window opens behind other windows. Activating it through the
global shortcut is the path that works, and the path users actually take.

## Milestone 1 status

Verified on Fedora 44, KDE Plasma, Wayland, GTK 4.22.

- [x] Centered launcher window with focused search field — see the caveat below.
- [x] Static in-memory result provider (`search::catalogue`) — since replaced
      as the primary source by real discovery, and now holding only the folders,
      links and Scene's own commands.
- [x] Query updates, grouped results, and the empty-result state — confirmed
      against a live window.
- [x] Repeated activation has no duplicate windows or stale state — three extra
      `scene` invocations left one process, one window and a cleared query.
- [x] UI styling can evolve without changing search or action logic.
- [x] Keyboard-only smoke path — covered by the UI smoke suite added in
      Milestone 4.5, which drives the whole contract against real widgets.

Seventy-six tests cover ranking determinism, grouping, case and whitespace
handling, category labelling, provider isolation, package-name validation,
capability detection, adapter argument vectors, history bounds and
persistence, launch watching, and the action outcomes for missing executables,
cancellation, output capture, accepted exit codes, and timeouts.
Ranking tests run against a fixture rather than the live index, so they do not
depend on what happens to be installed. The launcher starts with no GTK or CSS
warnings on stderr.

Out of scope here, by design: the integration and confirmation framework
(Milestone 3).

### Window position

Scene draws its own undecorated, rounded surface, but a Wayland client cannot
place its own window. On KDE Plasma the compositor decides, and centring is a
KWin placement setting rather than something Scene can assert. Until Milestone
5 introduces a real desktop-integration path, set:

*System Settings → Window Management → Window Behavior → Advanced →
Window placement: **Centered***

or add a window rule for `scene`. This is a genuine capability boundary, not a
missing feature — Scene reports what it can do rather than simulating it.

### Appearance

Scene renders a dark overlay surface on every theme, because that is what it
is. It picks up two system preferences at startup: animations are dropped when
`gtk-enable-animations` is off, and a high-contrast palette is used when a
HighContrast theme is active.

## Where this is going

`docs/krunner-parity.md` records what KRunner actually ships on Plasma 6.7 —
31 runners, read off this machine — and the P1-P6 track to match and exceed
it. Milestone 2 and Milestone 4.5 together complete parity item **P1** except
for a desktop entry's additional actions; Milestone 4 completes most of
**P5**, whose remaining gap is showing installed and available packages as one
merged result rather than separate rows.

## Tests

```sh
cargo test
cargo fmt --check
cargo clippy --all-targets
```
