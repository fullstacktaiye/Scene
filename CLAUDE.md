# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Scene is a keyboard-first Linux launcher: one Rust binary against native GTK 4,
KDE Plasma / Wayland first. `PRODUCT_PLAN.md` is the product authority and
milestone tracker; `AGENTS.md` holds the contributor conventions (commit style,
naming, module ownership) and applies here too. `docs/krunner-parity.md` and
`docs/copilot-key.md` record verified machine findings, not aspirations —
correct them rather than working around them in code.

The repository is at **Milestone 3** (safe command and integration framework).

## Commands

```sh
cargo run --release                     # build and launch
cargo test                              # 24 hermetic unit tests
cargo test ranking_is_stable_across_runs  # one test by name
cargo test search::tests                # one module's tests
cargo fmt --check
cargo clippy --all-targets
desktop-file-validate data/dev.scene.Scene.desktop   # after editing the entry
```

GTK4 development headers are a prerequisite (`gtk4-devel` / `libgtk-4-dev` /
`gtk4`). `SCENE_DIRECTORY=/path scene` overrides the configured-directory
integration.

## Architecture

One dependency (gtk4), Rust 2024, six modules with a deliberate one-way flow:

```
integrations ─┐
              ├─> search::index() ─> ui (renders) ─> actions (executes) ─> system
apps ─────────┘                                                   (subprocess only)
```

- **`search`** owns typed `Item`s, deterministic matching, ranking, grouping,
  and the static built-in catalogue. It never executes anything and never
  touches GTK widgets.
- **`ui`** renders what `search` produced and reports what `actions` returned.
  It decides neither. All appearance lives in `src/style.css`.
- **`actions`** owns `Action`, `ExecutionPolicy`, and `Outcome`. Results carry
  typed action values, never shell strings.
- **`system`** is intentionally the *only* module that runs a generic
  executable, from a fixed `CommandSpec` built by a provider — never from query
  text.
- **`apps`** discovers desktop entries via GIO's `AppInfo` (so `NoDisplay`,
  `Hidden`, `OnlyShowIn`, `NotShowIn` are honoured for free), then reads
  `GenericName`/`Keywords`/`Categories` from the entry with `glib::KeyFile`
  because the Rust bindings do not expose `GDesktopAppInfo`.
- **`main`** owns lifecycle only: one `Rc<ui::Launcher>` for the process, so a
  second activation re-presents and resets the existing window.

`PRODUCT_PLAN.md` §5 names two modules that do not exist yet — `platform`
(session/shortcut/workspace capability) arrives with Milestone 5, and the
distro capability adapters split out of `integrations` at Milestone 4.

### The safety boundary

This is the invariant most likely to be broken by a careless change:

1. Search text is never interpreted as a command. A provider registers a fixed
   program and arguments; nothing assembles one from the query.
2. `ExecutionPolicy` has three arms with structurally different paths.
   `ReadOnly` runs on a worker thread with a timeout, null stdin, bounded
   stdout/stderr, and a `CancellationToken`. `Detached` spawns a graphical
   program and does not wait. `Mutating` cannot be started by `actions::start`
   at all — only `actions::start_confirmed`, reached solely from the frozen
   `pending_confirmation` payload in `ui::Launcher`, and only via Enter. A
   result-row click deliberately cannot confirm a mutation, and a query change
   cannot substitute a different one.
3. A provider error becomes one local `unavailable_item` result. It must never
   remove another provider's results or break the launcher shell.
4. Failures are distinguished, not collapsed: unavailable tool, permission
   denied, timeout, cancellation, non-zero exit, spawn failure.

### Adding an integration

Implement `Integration` (metadata + `search(&Config)`) in `src/integrations.rs`
and add it to the `providers` array in `index()` — the array is a fixed-size
`[&dyn Integration; N]`, so bump `N`. Detect the executable with
`executable_on_path` rather than inferring it from a distribution name. No
change to `search`, `ui`, or `style.css` should be needed; that the UI stays
untouched is the Milestone 3 acceptance criterion, not just a convenience.

### Ranking

Group order comes from `Kind::priority()`, then score, then provider order —
`hits.sort_unstable()` over `(priority, -score, index)`, so ties resolve
reproducibly. Within an item, a title hit outranks a keyword hit (−40) which
outranks a description hit (−80). `fuzzy` is a subsequence matcher rewarding
word starts and adjacent runs. Any ranking change needs a fixture-based test.

### Outcome rendering

`Outcome` drives four exhaustive `match`es (`message`, `prefix`, `icon`,
`tone`) plus `should_dismiss`. Adding a variant means updating all of them and
the `.status.{ok,info,warn,error}` classes in `style.css`;
`every_outcome_states_itself_in_words` guards the first part.

## Testing

Tests are `#[cfg(test)] mod tests` blocks inside each module and must stay
hermetic: rank against `search::tests::fixture`, not the machine's installed
applications, and exercise subprocess behaviour through `system::tests::
fake_program` (a temporary `#!/bin/sh` script), never the host package
database. There is no UI test harness yet — GTK/Wayland behaviour is checked
manually, and the plan requires stating which verification was manual.

## Platform realities to respect

- A Wayland client cannot place its own window; centring is a KWin setting.
  Scene reports this limit rather than simulating placement.
- Launching passes the display's `AppLaunchContext` so the new window gets the
  activation token. The corollary: `scene` started from a shell has no token
  and opens behind other windows — the global shortcut is the real path.
- The Copilot key is a `Shift+Super+F23` chord emitting `XF86Assistant`, which
  Qt cannot bind. See `docs/copilot-key.md` before promising support for it.
