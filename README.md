# Scene

A fast, keyboard-first Linux launcher. See [`PRODUCT_PLAN.md`](PRODUCT_PLAN.md)
for what Scene is meant to become.

This repository currently contains **Milestone 3 — safe command and
integration framework**.

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

Scene registers as a single instance. Running `scene` again while it is already
running re-presents the existing window with a cleared query, rather than
opening a second one.

## Using it

| Input | Behaviour |
| --- | --- |
| Printable text | Filters and re-ranks the results |
| Up / Down | Moves the selection, wrapping at either end |
| Enter | Runs the selected result |
| Escape | Clears the query; on an empty query, hides the launcher |
| Click | Selects and runs, through the same path as Enter |
| Ctrl+Q | Quits Scene |

## Layout

```text
src/
├── main.rs     process lifecycle, activation, window reuse
├── apps.rs     installed application discovery
├── search.rs   matching, ranking, grouping, the built-in result set
├── actions.rs  typed actions and their outcomes
├── ui.rs       window, search field, result list, footer
└── style.css   the launcher's appearance
```

`ui.rs` renders what `search` produced and reports what `actions` returned. It
never decides either, so the styling can change without touching the logic.

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

The package integration selects `apt-cache`, `dnf`, or `pacman` only when its
executable is found on `PATH`, then runs its `--version` query. It performs no
package mutation; package search and mutating adapters belong to Milestone 4.

Automated verification covers the provider-error isolation, configuration,
confirmation decision, cancellation-before-spawn, bounded output, captured
stdout/stderr, and timeout behavior using temporary fake executables.

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
- [ ] Recent/frequent ranking — deliberately deferred. The plan requires the
      deterministic baseline first, and that has only just landed.

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
- [ ] Keyboard-only smoke path — Up/Down, Enter and Escape are wired and their
      logic is unit-tested, but this Wayland session has no input-injection
      tool, so nobody has driven them from a real keyboard yet.

Twenty-four unit tests cover ranking determinism, grouping, case and
whitespace handling, category labelling, provider isolation, and the action
outcomes for missing executables, cancellation, output capture, and timeouts.
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
it. Milestone 2 completes parity item **P1**.

## Tests

```sh
cargo test
cargo fmt --check
cargo clippy --all-targets
```
