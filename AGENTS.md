# Repository Guidelines

## Project Structure & Module Organization

Scene is a single Rust/GTK4 launcher binary. Keep process setup and GTK
application lifecycle in `src/main.rs`; keep each concern in its existing
module:

- `src/apps.rs` discovers desktop applications through GIO.
- `src/search.rs` owns typed results, deterministic matching, ranking, and
  built-in catalogue entries.
- `src/actions.rs` owns typed actions and execution outcomes; do not pass shell
  command strings through search or UI code.
- `src/ui.rs` renders state and connects GTK signals without making search or
  execution decisions. Styling belongs in `src/style.css`.
- `src/integrations.rs` holds the provider registry and typed configuration;
  `src/packages.rs` the distro capability adapters; `src/platform.rs` the
  desktop-session and shortcut observations; `src/system.rs` the only generic
  subprocess execution.
- `src/measure.rs` backs `scene --measure` and nothing else. Keep measurement
  code there rather than in the modules it measures.

Installed data is in `data/`: the desktop entry, the AppStream metainfo,
Scene's own icon, and the `scene.1` manual page. Packaging definitions are in
`packaging/`, one per distribution family, driven by `scripts/package.sh`. A
file that a package installs has to be installed by all three definitions and
by `scripts/install-user.sh`.

Product direction lives in `PRODUCT_PLAN.md`; parity, packaging, measurement
and release notes live under `docs/`.

## Build, Test, and Development Commands

Install GTK4 development headers first (for example, `sudo dnf install
gtk4-devel` on Fedora), then use Cargo from the repository root:

```sh
cargo run --release       # build and launch the optimized launcher
cargo test                # run hermetic unit tests
cargo fmt --check         # verify Rust formatting
cargo clippy --all-targets # lint production and test code
./scripts/package.sh      # build the distro packages in containers
```

Run `cargo fmt` before committing when formatting is needed. Validate edits to
the desktop entry with `desktop-file-validate data/dev.scene.Scene.desktop`
when its metadata changes.

To exercise a change through the desktop's global shortcut rather than a
terminal, run `./scripts/install-user.sh`. A `cargo build` alone will not do
it: the shortcut runs the installed binary, and a resident single-instance
process keeps serving the old code until it is stopped.

## Coding Style & Naming Conventions

Use Rust 2024 and rustfmt defaults (four-space indentation). Follow existing
Rust naming: `PascalCase` types and enum variants, `snake_case` functions and
modules, and descriptive lower-case test names such as
`ranking_is_stable_across_runs`. Prefer small, typed APIs and exhaustive
`match`es for action/outcome behavior. Preserve the separation between search,
actions, and UI, especially the no-arbitrary-shell-command safety boundary.

## Testing Guidelines

Place focused unit tests in a module's `#[cfg(test)] mod tests` block. Test
search behavior against fixed fixtures, not the machine's installed
applications, so results remain deterministic and hermetic. Add regression
tests for ranking, parsing, or action outcomes alongside the changed logic.
Manual GTK/Wayland checks are useful for visual or activation behavior, but do
not replace automated tests; note any environment-dependent verification in the
pull request.

## Commit & Pull Request Guidelines

Use concise, imperative commit subjects consistent with history, e.g.
`Centre the application icon in its tile` or `Milestone 2: application discovery and launching`.
Keep commits focused. Pull requests should explain the user-visible change,
link the issue or milestone when applicable, list commands run, and include a
screenshot for visible UI changes. Call out platform assumptions, GTK version
requirements, and any manual verification separately.
