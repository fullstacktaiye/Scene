# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Scene is a keyboard-first Linux launcher: one Rust binary against native GTK 4,
KDE Plasma / Wayland first. `PRODUCT_PLAN.md` is the product authority and
milestone tracker; `AGENTS.md` holds the contributor conventions (commit style,
naming, module ownership) and applies here too. `docs/krunner-parity.md`,
`docs/copilot-key.md` and `docs/fedora-packaging.md` record verified machine
findings, not aspirations — correct them rather than working around them in
code.

The repository is at **Milestone 4.5** (overdue milestone items).

## Commands

```sh
cargo run --release                     # build and launch
cargo test                              # 69 tests: 68 hermetic, 1 UI smoke
cargo test ranking_is_stable_across_runs  # one test by name
cargo test search::tests                # one module's tests
cargo test ui::tests -- --nocapture     # the UI smoke suite alone
cargo fmt --check
cargo clippy --all-targets
desktop-file-validate data/dev.scene.Scene.desktop   # after editing the entry
./scripts/install-user.sh               # make the desktop shortcut run this build
```

GTK4 development headers are a prerequisite (`gtk4-devel` / `libgtk-4-dev` /
`gtk4`). `SCENE_DIRECTORY=/path scene` overrides the configured-directory
integration, and `SCENE_HISTORY=off scene` turns recent/frequent ranking off.

`cargo build` does not change what the desktop shortcut starts. That runs the
installed binary through a desktop entry, and Scene is single-instance, so a
resident process keeps serving the old code even after the file is replaced.
`scripts/install-user.sh` handles both, and regenerates the user entry from
`data/dev.scene.Scene.desktop` so the deployed copy cannot drift from the one
in the repository. Never hand-edit the copy under `~/.local/share/applications`.

The UI smoke suite opens a real window for a fraction of a second while
`cargo test` runs, which is expected. Where there is no display it prints
`skipping the UI smoke suite` and passes.

## Architecture

One dependency (gtk4), Rust 2024, seven modules with a deliberate one-way flow:

```
apps ─────────┐
packages ─────┼─> integrations ─> search (ranks) ─> ui (renders)
              │                                       │
              └───────────────────────────────────────┴─> actions ─> system
                                                             (subprocess only)
```

`search` sees two streams: `index()` for what exists regardless of the query,
and `answers(query)` for results that exist only while a query asks for them.
Both are ranked by the same matcher, in one list.

- **`search`** owns typed `Item`s, deterministic matching, ranking, grouping,
  the static built-in catalogue, and `History` — the recent/frequent
  adjustment, which is an explicit argument to `search()` rather than ambient
  state, so the same query, items and history always give the same order. It
  never executes anything and never touches GTK widgets.
- **`ui`** renders what `search` produced and reports what `actions` returned.
  It decides neither. All appearance lives in `src/style.css`.
- **`actions`** owns `Action`, `ExecutionPolicy`, and `Outcome`. Results carry
  typed action values, never shell strings.
- **`system`** is intentionally the *only* module that runs a generic
  executable, from a fixed `CommandSpec` built by a provider — never from query
  text. It also owns executable discovery (`locate`), so capability detection
  never infers a tool from a distribution name.
- **`packages`** holds the distro capability adapters. It runs nothing: an
  adapter answers with a `Plan` — resolved program, argument vector, timeout,
  output limit, accepted exit codes — that `actions` executes under the usual
  policy.
- **`apps`** discovers desktop entries via GIO's `AppInfo` (so `NoDisplay`,
  `Hidden`, `OnlyShowIn`, `NotShowIn` are honoured for free), then reads
  `GenericName`/`Keywords`/`Categories` from the entry with `glib::KeyFile`
  because the Rust bindings do not expose `GDesktopAppInfo`.
- **`main`** owns lifecycle only: one `Rc<ui::Launcher>` for the process, so a
  second activation re-presents and resets the existing window.

`PRODUCT_PLAN.md` §5 names one module that does not exist yet: `platform`
(session/shortcut/workspace capability), which arrives with Milestone 5.

### The safety boundary

This is the invariant most likely to be broken by a careless change:

1. Search text is never interpreted as a command. A provider registers a fixed
   program and arguments; nothing assembles one from the query.
2. `ExecutionPolicy` has three arms with structurally different paths.
   `ReadOnly` runs on a worker thread with a timeout, null stdin, bounded
   stdout/stderr, and a `CancellationToken`. `Detached` spawns a graphical
   program and watches it for `actions::START_WATCH` (400 ms) — long enough to
   catch one that dies at once, and never cancellable, because Escape must not
   kill what the user just started. `Mutating` cannot be started by `actions::start`
   at all — only `actions::start_confirmed`, reached solely from the frozen
   `pending_confirmation` payload in `ui::Launcher`, and only via Enter. A
   result-row click deliberately cannot confirm a mutation, and a query change
   cannot substitute a different one.
3. A provider error becomes one local `unavailable_item` result. It must never
   remove another provider's results or break the launcher shell.
4. Failures are distinguished, not collapsed: unavailable tool, permission
   denied, timeout, cancellation, non-zero exit, spawn failure.
5. Scene never reports an outcome it did not observe. `Outcome::Succeeded`
   means the work was watched and finished; `Outcome::Started` means the
   program was handed over and is no longer being watched — a desktop launch,
   or a program still alive when `START_WATCH` elapsed. Do not collapse the
   two.

### Adding an integration

Implement `Integration` in `src/integrations.rs` and add it to the `PROVIDERS`
array — it is a fixed-size `[&dyn Integration; N]`, so bump `N`. Detect the
executable with `system::executable_on_path` rather than inferring it from a
distribution name. No change to `search`, `ui`, or `style.css` should be
needed; that the UI stays untouched is the Milestone 3 acceptance criterion,
not just a convenience.

The trait has two halves. `search(&Config)` returns what exists regardless of
the query and is collected once. `answer(query, &Config)` returns results for
one specific query and runs on the GTK thread for every keystroke, so it may
*build* a command but must never run one. An answer carries the query itself as
its first keyword, which is what makes a generated result matchable by the same
ranker with no special case in `search`.

### Adding a distro adapter

Implement `Adapter` in `src/packages.rs` — `family`, `signature` (the
executable that identifies the family), and `command` for each `Capability` —
and add it to `ADAPTERS`. A capability's `Recipe` names the one executable it
needs; presence on `PATH` is what makes it real. `/etc/os-release` only orders
which adapter is probed first and can never make a capability available.
Mutating recipes are marked `.elevated()` and route through `pkexec`; there is
no `sudo` fallback. If a tool answers a question with an exit status, say so
with `.accepting(...)` and record the machine you verified it on.

Package names reach an argument vector only through `Term::parse`, which allows
`[A-Za-z0-9._+-]`, bounds the length, and rejects a leading `-` that a tool
would read as an option. Nothing else may put query text into a command.

### Ranking

Group order comes from `Kind::priority()`, then score, then provider order —
`hits.sort_unstable()` over `(priority, -score, index)`, so ties resolve
reproducibly. Within an item, a title hit outranks a keyword hit (−40) which
outranks a description hit (−80). `fuzzy` is a subsequence matcher rewarding
word starts and adjacent runs. Any ranking change needs a fixture-based test.

With no query every item "matches" and the score carries no information, so
`Kind::resting_limit` trims each group to its top few — `Application` only,
because it is the one group whose size is discovered rather than chosen. A
query is never trimmed, so search still reaches everything. `ui` reports a
trimmed group as `5 of 81` in its heading by counting what it rendered against
what it was given; it does not know the rule.

`History` adds at most 24 (frequency) + 15 (recency) = 39 to a matched item's
score. That bound is load-bearing, not arbitrary: it sits below the 40-point
title/keyword gap, so use can lift a result within its group and can never
make a keyword match outrank a title match. Group order is untouched because
the bonus is inside the score, not the priority. State lives in
`$XDG_STATE_HOME/scene/history`, whose first line names its format version.

### Outcome rendering

`Outcome` drives four exhaustive `match`es (`message`, `prefix`, `icon`,
`tone`) plus `should_dismiss`. Adding a variant means updating all of them and,
unless it reuses an existing tone, the `.status.{ok,info,warn,error}` classes
in `style.css`; `every_outcome_states_itself_in_words` guards the first part.
A non-dismissing outcome arriving after the launcher has closed re-presents
the window, so a watched launch that fails is never reported to nobody.

## Testing

Tests are `#[cfg(test)] mod tests` blocks inside each module and must stay
hermetic: rank against `search::tests::fixture`, not the machine's installed
applications, and exercise subprocess behaviour through `system::tests::
fake_program` (a temporary `#!/bin/sh` script), never the host package
database.

`ui.rs` carries the UI smoke suite: one test, because GTK must be used from
the thread that initialised it. It drives the whole keyboard contract against
real widgets, entering keys at `Launcher::key` — the same method the window's
key controller calls. What it cannot prove is the compositor delivering a
physical key press to that controller, which stays a manual check. Keep it one
test function, keep it skipping cleanly without a display, and keep its
history disabled so it never writes to the user's state file.

## Platform realities to respect

- A Wayland client cannot place its own window; centring is a KWin setting.
  Scene reports this limit rather than simulating placement.
- Launching passes the display's `AppLaunchContext` so the new window gets the
  activation token. The corollary: `scene` started from a shell has no token
  and opens behind other windows — the global shortcut is the real path.
- The Copilot key is a `Shift+Super+F23` chord emitting `XF86Assistant`, which
  Qt cannot bind. See `docs/copilot-key.md` before promising support for it.
