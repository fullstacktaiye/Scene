# Scene

A fast, keyboard-first Linux launcher. See [`PRODUCT_PLAN.md`](PRODUCT_PLAN.md)
for what Scene is meant to become.

This repository currently contains **Milestone 1 — the launcher shell**.

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
├── search.rs   matching, ranking, grouping, the static result set
├── actions.rs  typed actions and their outcomes
├── ui.rs       window, search field, result list, footer
└── style.css   the launcher's appearance
```

`ui.rs` renders what `search` produced and reports what `actions` returned. It
never decides either, so the styling can change without touching the logic.

## Milestone 1 status

Verified on Fedora 44, KDE Plasma, Wayland, GTK 4.22.

- [x] Centered launcher window with focused search field — see the caveat below.
- [x] Static in-memory result provider (`search::catalogue`).
- [x] Query updates, grouped results, and the empty-result state — confirmed
      against a live window.
- [x] Repeated activation has no duplicate windows or stale state — three extra
      `scene` invocations left one process, one window and a cleared query.
- [x] UI styling can evolve without changing search or action logic.
- [ ] Keyboard-only smoke path — Up/Down, Enter and Escape are wired and their
      logic is unit-tested, but this Wayland session has no input-injection
      tool, so nobody has driven them from a real keyboard yet.

Twelve unit tests cover ranking determinism, grouping, case and whitespace
handling, and the action outcomes for a missing executable and a missing path.
The launcher starts with no GTK or CSS warnings on stderr.

Out of scope here, by design: global activation (Milestone 5), real application
discovery (Milestone 2), and the integration and confirmation framework
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

## Tests

```sh
cargo test
cargo fmt --check
cargo clippy --all-targets
```
