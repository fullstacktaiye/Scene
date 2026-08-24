# Scene Product Plan

## 1. Product definition

Scene is a fast, keyboard-first Linux launcher and system workspace tool. It
will begin as a focused launcher for KDE Plasma and grow toward a practical,
understandable replacement for common KRunner workflows.

Scene is intended to make the next useful action easy to find and safe to
execute. It combines application discovery, fuzzy search, desktop actions,
small integrations, and carefully bounded system capabilities in one centered
launcher surface.

The initial implementation is a single Rust executable with a native GTK4
interface. The first desktop target is KDE Plasma. Debian-family, Fedora, and
Arch support will be implemented through capability adapters, so the presence
of a distribution label never substitutes for detecting the executable or
desktop capability that is actually available.

### What Scene is

- A low-overhead launcher that can be activated globally.
- A keyboard-first search surface for applications, actions, and integrations.
- A small system workspace tool with visible execution state and errors.
- A KDE-first product with explicit capability reporting on other sessions.
- A foundation for named workspace/application groups, initially called
  Stage Manager.

### What Scene is not

- A large dashboard, permanent sidebar, or general-purpose desktop shell.
- An opaque plugin marketplace or a mechanism for arbitrary unreviewed code to
  run with implicit privileges.
- A package manager that assumes one distribution, command, or privilege model.
- A universal window-layout engine in its first Stage Manager prototype.
- A promise that every Linux desktop or Copilot-key implementation behaves the
  same way.

### Product outcome

The first release should prove a reliable launcher core: activate Scene, type a
query, understand the results, launch an application or perform a safe action,
and recover clearly when something is unavailable. Later releases may add
broader system and workspace features without making that core harder to
understand.

## 2. Product principles

1. **Fast to useful.** Activation, focus, indexing, and search should feel
   immediate. Work that may block belongs off the UI thread and has explicit
   cancellation or timeout behavior.
2. **Keyboard first, mouse welcome.** Every primary launcher journey must work
   with the keyboard, while pointer interaction remains predictable and
   accessible.
3. **Small, composable capabilities.** Integrations should have narrow
   contracts, explicit metadata, and isolated failures. Adding an integration
   must not require rewriting the search UI.
4. **Visible consequences.** Scene shows what an action will do, whether it is
   running, its output or result, and why it failed.
5. **Safe by default.** System-changing actions are typed, bounded, and
   explicitly confirmed. Scene does not silently elevate privileges or turn
   search text into an unrestricted shell command.
6. **Capability over assumptions.** Detect actual desktop and executable
   capabilities. Report unsupported behavior honestly and keep unrelated
   launcher features available.
7. **Native and maintainable.** Prefer Rust and native desktop primitives for
   process, input, lifecycle, and platform integration. Keep platform-specific
   behavior behind clear boundaries.
8. **Accessible and restrained.** Readable typography, sufficient contrast,
   focus visibility, reduced-motion support, and high-contrast behavior are
   product requirements, not polish deferred to the end.

## 3. Initial user journeys

### Launch an application

1. The user activates Scene with the configured shortcut.
2. A centered launcher appears with the search field focused.
3. The user types part of an application name or keyword.
4. Scene presents deterministic, fuzzy-ranked results grouped by type.
5. The user moves with Up/Down or chooses with the pointer.
6. Enter launches the selected desktop application.
7. Scene closes or returns to its configured post-launch state. A launch error
   remains understandable and actionable in the UI.

### Perform a safe system action

1. The user searches for a built-in integration action such as system
   information, opening a configured directory, or querying package metadata.
2. The result explains the action and any relevant constraints.
3. Scene runs it with a bounded process policy.
4. Progress, output, cancellation, timeout, or failure is visible.

### Attempt a mutating action

1. The user selects an action that installs, removes, upgrades, reboots,
   shuts down, changes networking, or modifies system configuration.
2. Scene presents an explicit confirmation describing the consequence and
   target.
3. Nothing changes unless the user confirms in the dedicated confirmation
   interaction.
4. Scene reports the final result, including permission or missing-tool
   failures. The action never implies success from merely starting a process.

### Switch a work context (future)

1. The user searches for a named Scene workspace/application group.
2. Scene shows the group and its associated applications.
3. The user selects it to request a desktop-specific workspace switch.
4. On success, the context changes. On failure or unsupported capability, the
   current session is left intact and Scene explains the limitation.

## 4. Interaction and visual direction

Scene takes inspiration from Raycast’s focused command surface: a single,
centered launcher with strong hierarchy and little persistent chrome. The
visual system should feel calm and deliberate rather than flashy.

### Launcher surface

- One centered window with a clear search field at the top.
- A compact result list with visible result type, title, description, and
  optional icon or category.
- A strong selected-result state that is visible without relying on color
  alone.
- Empty, loading, unavailable, error, and success states that explain what is
  happening.
- Subtle borders, a modest radius, restrained shadows, and restrained
  animation.
- No large dashboard or permanent sidebar in the initial launcher.

GTK4 supplies application lifecycle, input handling, focus management, and
desktop integration primitives. Window presentation, global activation, and
compositor-dependent behavior remain desktop-integration concerns and must be
validated on KDE Plasma in practice. See the [GTK application
documentation](https://docs.gtk.org/gtk4/class.Application.html) and [GTK
input handling documentation](https://docs.gtk.org/gtk4/input-handling.html).

### Keyboard model

The baseline interaction contract is:

| Input | Behavior |
| --- | --- |
| Printable text | Update the query and refresh ranked results |
| Up / Down | Move the selected result, with sensible wrapping or boundary behavior |
| Enter | Execute the selected result or open its confirmation step |
| Escape | Close the launcher; cancel an in-progress interaction where appropriate |
| Tab / Shift+Tab | Move through exposed controls without trapping focus |
| Pointer click | Select and execute using the same action path as the keyboard |

The exact shortcut for activation is configurable. The launcher must refocus
the search field on each activation and must not accumulate duplicate windows,
stale queries, or stale selection state across repeated open/close cycles.

### Accessibility requirements

- Correct accessible names, roles, and descriptions for the search field,
  result list, selected result, status messages, and confirmation controls.
- Visible keyboard focus and selected state independent of hue alone.
- Readable text sizing and contrast, including high-contrast themes.
- Respect for reduced-motion preferences.
- Error and unavailable states expressed in text, not only icons or color.

## 5. Technical architecture

The initial application is one Rust binary with modules organized around
responsibilities rather than individual integrations:

```text
app
├── ui            GTK window, search field, result list, styling, focus
├── search        query parsing, fuzzy matching, ranking, result groups
├── actions       typed actions, policy, confirmation, result/error state
├── integrations  registered providers with explicit metadata and behavior
├── packages      distro capability adapters and their package operations
├── platform      desktop session, shortcuts, windows, workspaces
└── system        command discovery, adapters, subprocesses, timeouts, output
```

### `app`

Owns process lifecycle, activation, configuration loading, application state,
and the coordination of services. It ensures repeated activation reuses the
intended launcher state rather than creating competing windows.

### `ui`

Owns GTK widgets, layout, visual styling, focus, keyboard navigation, and
presentation of result, progress, confirmation, and error states. It consumes
search and action contracts; it does not discover commands or embed distro
conditionals.

### `search`

Owns query parsing, fuzzy matching, deterministic ranking, provider priority,
result grouping, and later history/frequency adjustments. Search providers
return typed results with stable identifiers and enough metadata for the UI to
render them.

### `actions`

Owns typed execution requests and their lifecycle. Every action carries:

- a stable identifier;
- a human-readable title and description;
- search keywords;
- an execution policy;
- a result or error message; and
- whether confirmation is required.

An action result must distinguish at least pending, succeeded, failed,
cancelled, timed out, and unavailable states. The UI receives those states
without needing to know how a provider implemented the action.

### `integrations`

Each integration declares an identifier and display metadata, then provides a
search provider and action provider. It may optionally provide configuration.
Its execution result, error behavior, and cancellation behavior are explicit.
An integration failure is isolated and must not prevent unrelated providers or
the launcher shell from working.

### `platform`

Abstracts desktop-session detection, global activation, window behavior,
application launching, and workspace operations. KDE Plasma is the first
complete target. Unsupported or unverified capabilities are represented as
such, rather than simulated as successful behavior.

### `system`

Owns executable discovery, subprocess execution, timeouts, cancellation,
output capture, and structured error reporting. Package-manager operations are
separate from generic command execution and are routed through capability
adapters.

## 6. Action safety model

Scene must not interpret arbitrary search text as an unrestricted shell command.
Actions are registered, typed, and executed through known policies. A generic
command capability, if introduced later, must still define its executable,
arguments, environment, working directory, timeout, output limits, and
permission behavior.

The following actions require explicit confirmation:

- installing, removing, or upgrading packages;
- rebooting or shutting down;
- changing networking;
- modifying system configuration; and
- any other action whose declared policy changes durable system state.

Confirmation must identify the operation and target in user-readable language.
It must not be bypassed by keyboard ambiguity, stale selection, or a provider
that changes its payload after the result was displayed. Privilege escalation,
when a platform action genuinely requires it, must be explicit and use the
desktop’s supported mechanism; Scene must not silently assume `sudo` or a
particular authentication agent.

Long-running work must be cancellable or time-limited. Output should be
bounded, safely rendered, and associated with the action that produced it.
Failures should distinguish unavailable executables, permission failures,
timeouts, cancellation, non-zero exit status, malformed provider data, and
unsupported desktop capabilities where practical.

## 7. Platform and distribution strategy

### KDE Plasma first

KDE Plasma is the first desktop target for global activation, application
launch, startup integration, and the eventual Stage Manager prototype. KDE or
compositor-specific behavior belongs in `platform`, with capability checks and
manual validation at the desktop boundary.

### Global activation and Copilot key

Scene will provide a configurable fallback shortcut and a KDE Plasma-first
activation path. Copilot-key support is best effort:

- detect it only when Linux exposes it as an observable input event or desktop
  action;
- show the active shortcut and detection status in settings;
- provide a manual shortcut-recording fallback when the session supports it;
- never claim Copilot-key support without observing and verifying the key; and
- report unsupported hardware or session combinations clearly.

Scene must remain usable from the fallback shortcut when Copilot-key support
is absent.

### Distro capability adapters

Adapters share one interface and expose capabilities such as package search,
package metadata lookup, installed-package lookup, update availability, and
optional mutation actions behind confirmation.

| Family | Preferred capability tools | Detection rule |
| --- | --- | --- |
| Debian / Ubuntu | `apt`, `apt-cache` | Confirm the executable and usable invocation; distribution metadata alone is insufficient |
| Fedora | `dnf` | Confirm the executable, permissions, and supported command behavior |
| Arch | `pacman` | Confirm the executable, permissions, and supported command behavior |

The adapter must return an understandable unavailable-capability result when a
tool is missing, unusable, or unsupported. Core launching and search must
continue when package capabilities are unavailable. Validation is required on
at least one Debian or Ubuntu-family system, Fedora system, and Arch system.

## 8. Milestones and completion checklists

Each milestone has a user-visible outcome and a completion checklist. A later
milestone must not be considered complete merely because its code exists; the
listed behavior and validation evidence are part of completion.

### Milestone 0 — Product and interaction foundation

**User-visible outcome:** The project has a shared definition of Scene, a
coherent launcher interaction model, and explicit safety and platform
boundaries.

**Checklist:**

- [x] `PRODUCT_PLAN.md` explains what Scene is and is not.
- [x] Product principles, initial user journeys, and visual direction are
      documented.
- [x] Keyboard, accessibility, integration, safety, and KDE-first assumptions
      are explicit.
- [x] The full vision and deferred features are separated from the first
      implementation scope.
- [x] Every later milestone has an outcome and completion checklist.

### Milestone 1 — Launcher shell

**User-visible outcome:** A local GTK4 application opens a centered launcher,
focuses search, accepts queries, navigates results, executes static in-memory
results, and closes reliably.

**Checklist:**

- [x] Centered launcher window with focused search field.
- [x] Static in-memory result provider.
- [x] Query updates, Up/Down navigation, Enter, Escape, and empty-result state.
- [x] Repeated activation has no duplicate windows or stale state. Re-checked
      during Milestone 4.5: invoking `scene` again while it was running exited
      0 immediately and left exactly one process.
- [x] UI styling can evolve without changing search or action logic.
- [x] Keyboard-only smoke path is reliable. Closed in Milestone 4.5 by the UI
      smoke suite in `src/ui.rs`; see that milestone for what it proves and
      what remains a manual check.

### Milestone 2 — Application discovery and launching

**User-visible outcome:** Scene finds installed graphical applications and
launches them through the desktop application model.

**Checklist:**

- [x] Discover Linux desktop entries asynchronously or incrementally.
- [x] Render application icons, names, descriptions, and categories where
      available.
- [x] Add deterministic fuzzy search and ranking.
- [x] Present launch failures in the UI.
- [x] Add recent/frequent ranking only after baseline ranking is deterministic.
      Closed in Milestone 4.5: `search::History`, bounded so it cannot
      overturn the baseline, and switched off with `SCENE_HISTORY=off`.
- [x] Include a valid `.desktop` entry for Scene itself and follow the
      [desktop-entry specification](https://specifications.freedesktop.org/desktop-entry-spec/latest/).
- [x] Check packaging behavior against [Fedora desktop application packaging
      guidance](https://docs.fedoraproject.org/en-US/packaging-guidelines/).
      Closed in Milestone 4.5; the result is `docs/fedora-packaging.md`.

### Milestone 3 — Safe command and integration framework

**User-visible outcome:** A provider can add useful, bounded actions without
changing the launcher UI, and failures stay localized.

**Checklist:**

- [x] Define integration metadata, search, action, configuration, result,
      error, and cancellation contracts.
- [x] Add integrations for opening a terminal, showing system information,
      searching or launching installed applications, opening a configured
      directory, and querying the detected package manager without mutation.
- [x] Add typed execution policies, timeouts, output capture, and cancellation.
- [x] Show confirmation for mutating actions.
- [x] Isolate provider failures from the rest of the result set.
- [x] Prove the contract by adding an integration without changing search UI
      code.

### Milestone 4 — Distro capability adapters

**User-visible outcome:** The same package-related Scene action uses the
appropriate available tool on Debian-family, Fedora, and Arch systems.

**Checklist:**

- [x] Implement shared adapters for Debian/Ubuntu-family, Fedora, and Arch.
- [x] Expose package search, metadata, installed-package, and update
      capabilities where supported.
- [x] Gate optional mutation actions behind confirmation.
- [x] Detect executable capabilities rather than relying only on distro name.
- [x] Report missing tools, permission issues, and unsupported operations
      clearly.
- [x] Verify at least one Debian/Ubuntu-family, Fedora, and Arch environment.
- [x] Verify core launcher behavior when an adapter is unavailable.

### Milestone 4.5 — Overdue milestone items

**Why this exists:** Milestones 0-4 are otherwise complete, but four checklist
items whose milestone has already passed are still open. They are collected
here rather than left scattered across finished milestones, so that "Milestone
4 is done" cannot quietly mean "done except for the parts nobody is tracking".
Recorded when Milestone 4 completed.

**User-visible outcome:** Every checklist item belonging to a completed
milestone is either finished or carries an explicit deferral naming the
milestone that will finish it.

The table below records the state when this milestone opened; what closed each
item is under **What was done** further down.

| Item | Originally due | Why it was still open |
| --- | --- | --- |
| Keyboard-only smoke path is reliable | Milestone 1 | The bindings are wired and their logic is unit-tested, but nobody has driven Up/Down, Enter and Escape from a real keyboard. This Wayland session has no input-injection tool, so the item needs a manual pass or the UI smoke harness that Milestone 8 also calls for. |
| Recent/frequent ranking | Milestone 2 | The plan gates it on a deterministic ranking baseline, which landed inside Milestone 2. The precondition has been met since; the work has not started, and no decision to defer it was recorded. |
| Fedora desktop-application packaging check | Milestone 2 | Never performed. `data/dev.scene.Scene.desktop` passes `desktop-file-validate`, which is a different and much weaker check than the Fedora packaging guidance. Its result feeds the Fedora package in Milestone 8. |
| Launch failure detail: non-zero exit | Milestone 3, via `docs/krunner-parity.md` P1 | A missing executable and a permission failure are distinguished. A launched or detached program that starts and *then* fails is not observed at all, because nothing waits on it — so Scene reports `Succeeded` for it. The parity plan assigned this gap to Milestone 3's bounded-execution work; Milestone 3 built that machinery without applying it to launches. |

**Checklist:**

- [x] Drive the keyboard-only path end to end on KDE Plasma — query, Up/Down,
      Enter, Escape, empty results, confirmation, and cancellation — and
      record the result. Add a UI smoke harness if manual validation keeps
      being the blocker.
- [x] Settle recent/frequent ranking: implement it over the deterministic
      baseline with a way to disable it, or move it to Milestone 6 with a
      stated reason. It must not stay implicit.
- [x] Check the desktop entry and the build against the [Fedora desktop
      application packaging
      guidance](https://docs.fedoraproject.org/en-US/packaging-guidelines/),
      and record what it requires that Scene does not yet do.
- [x] Observe a launched program's outcome, or say in the UI that Scene does
      not. A launch that starts and then fails must not report success.
- [x] Every remaining unticked item in Milestones 0-4 is either closed here or
      carries an explicit deferral with a target milestone.

**What was done, and what it proves.**

*Keyboard-only path.* Manual validation kept being the blocker — this Wayland
session has no input-injection tool — so the milestone's stated alternative
was taken: `src/ui.rs` now carries a UI smoke suite. It builds the real
launcher against a stated set of items, presents the window, and drives the
whole contract in order: activation focuses the search field, typing narrows
and re-ranks, Down and Up move the selection and wrap at both ends, a query
with no results shows the empty state and Enter on it does nothing, Enter runs
the selected result and reports it, Escape clears the query and then closes,
re-activation resets query, status and selection, a mutation asks before it
acts and a result-row click cannot answer for it, Escape withdraws the
confirmation, a second Enter confirms it, and a long action is cancelled from
the keyboard. It runs against real GTK widgets and real `Outcome`s, and skips
itself with a printed reason where there is no display.

The limit is stated rather than hidden: keys enter at `Launcher::key`, the
same method the window's key controller calls. What the suite does not prove
is the compositor delivering a physical key press to that controller.

That link was then driven by hand, which is what closes the item. **Manual
pass: 2026-08-24, Fedora 44 / KDE Plasma 6.7 / Wayland, real keyboard.** The
same sequence the suite runs: typing narrowed and re-ranked, Up and Down moved
the selection and wrapped at both ends, a query with no results showed the
empty state and Enter on it did nothing, Enter reported in the footer without
closing the launcher, Escape cleared the query and then closed it, an
`install` answer stopped at its confirmation and Escape withdrew it, and a
terminal launch reported and then dismissed.

The run left its own evidence in `$XDG_STATE_HOME/scene/history`, which
recorded `scene.reporting`, `packages.metadata` and `terminal.open` at
13:09:27, 13:09:46 and 13:10:46. The withdrawn `install` is *not* in that
file, which is the recording rule working: use is recorded when an action
starts, so a confirmation the user backed out of does not count as one. On the
next activation the terminal row ranked above where it had sat before, which
is the ranking adjustment observed end to end rather than only in a test.

*Recent/frequent ranking.* Implemented rather than deferred: `search::History`
records what was chosen and adjusts the score within a group. The adjustment
is deliberately bounded — at most 24 for frequency plus 15 for recency, which
is under the 40-point gap the matcher puts between a title hit and a keyword
hit — so use can lift a result within its group but can never overturn the
deterministic field order, and never crosses a group boundary. Ranking stays
deterministic in the sense the plan requires, because the history is an
explicit argument to `search`, not ambient state: the same query, items and
history always give the same order. It is switched off entirely with
`SCENE_HISTORY=off`, and Milestone 6 owns giving that a settings surface.
State lives in `$XDG_STATE_HOME/scene/history`, in a format whose first line
names its version, which is where Milestone 8's configuration migration will
start.

*Fedora packaging.* Performed and recorded in `docs/fedora-packaging.md`,
against the guidelines' own source rather than the rendered site. The desktop
entry and package independence already comply. Two gaps were closed outright
because they were missing rather than deferred: the repository had no `LICENSE`
file for `%license` to point at despite declaring MIT, and no AppStream
metainfo, which a GUI application is expected to install. What remains is
Milestone 8's: the spec file, an icon of Scene's own, screenshots, a release
tarball, and a mock build.

*Launch outcomes.* Both halves of the item are now true. Scene watches a
program it starts for 400 ms and reports the exit status if it dies inside
that window, so a launch that starts and then fails is reported as a failure
rather than a success — and when the launcher has already closed, the window
comes back to show it. For an installed application handed to the desktop
there is no handle and no exit status to read, so Scene says so instead of
claiming an outcome: `Outcome::Started` is a different answer from
`Outcome::Succeeded`, and the built-in "What Scene Reports" result states the
limit in words.

**Deferred, with a target milestone.** One item in this area is deliberately
not closed here and is not left implicit: `docs/krunner-parity.md` P1's
support for a desktop entry's declared additional actions ("New Private
Window", "Open a New Document"). It belongs with the inline-actions work in
**Milestone 6** / parity P6, where a result can carry several operations,
rather than as a special case in application discovery.

### Milestone 5 — Global activation and Copilot-key support

**User-visible outcome:** A user can activate Scene without a terminal and can
see whether the preferred key is actually supported.

**Checklist:**

- [ ] Configurable fallback shortcut.
- [ ] KDE Plasma-first activation path.
- [ ] Best-effort Copilot-key detection through observed input or desktop
      action.
- [ ] Settings view for active shortcut and detection status.
- [ ] Manual shortcut recording where supported.
- [ ] Clear unsupported-session reporting.
- [ ] Manual KDE validation of activation and repeated toggling.

### Milestone 6 — KDE replacement foundation

**User-visible outcome:** Scene covers common application-launching and basic
command workflows that users expect from KRunner, while remaining predictable
as providers grow.

**Checklist:**

- [ ] Multiple result categories.
- [ ] Prioritized search providers.
- [ ] Inline actions.
- [ ] Command history.
- [ ] Configurable providers.
- [ ] Basic settings and independent provider enable/disable controls.
- [ ] Startup integration.
- [ ] Verify common KRunner replacement workflows manually on KDE Plasma.

### Milestone 7 — Stage Manager prototype

**User-visible outcome:** A user can switch between at least two practical
named work contexts when the desktop session supports the required behavior.

**Checklist:**

- [ ] Create, rename, select, and delete named scenes.
- [ ] Associate applications with a scene.
- [ ] Switch scenes through the launcher.
- [ ] Isolate KDE/desktop-specific operations behind `platform`.
- [ ] Disable the feature when required compositor/session capabilities are
      unavailable.
- [ ] Ensure a failed switch leaves the current session intact.
- [ ] Document and manually validate the KDE behavior.

### Milestone 8 — Packaging and release quality

**User-visible outcome:** Scene can be installed, started, upgraded, and
validated reproducibly on its supported Linux targets.

**Checklist:**

- [ ] Fedora package.
- [ ] Debian package.
- [ ] Arch package.
- [ ] User-level autostart configuration.
- [ ] Versioned configuration migration.
- [ ] Reproducible development and packaging instructions.
- [ ] Focused unit, hermetic integration, and UI smoke suites.
- [ ] Startup, indexing, search-latency, and idle-memory measurements.
- [ ] Release documentation includes known desktop and capability limits.

## 9. Testing and acceptance standards

Testing should follow the capability boundaries and test user-visible failure
behavior, not only successful paths.

### Automated validation

- Unit tests for fuzzy ranking, query parsing, result grouping, capability
  detection, action validation, policy/confirmation decisions, and
  configuration migration.
- Hermetic subprocess integration tests using fake executables. Tests must not
  mutate the host package database or depend on a particular installed
  distribution tool.
- UI smoke tests for launch, focus, typing, selection, Enter, Escape,
  empty-result, confirmation, success, unavailable, and error states.
- Cancellation, timeout, non-zero exit, malformed output, and repeated
  activation tests.
- `cargo fmt --check`.
- `cargo test`.
- `cargo clippy --all-targets`.
- `git diff --check`.

### Manual validation

- KDE Plasma validation for global activation, window presentation, startup,
  and workspace behavior.
- Debian or Ubuntu-family, Fedora, and Arch validation for distro adapters.
- Validation on sessions where expected capabilities are absent, including
  behavior of the core launcher when optional integrations cannot load.
- Keyboard-only and high-contrast/reduced-motion checks.

## 10. Product measurements

The project should record measurements as features become real, using a
consistent environment and clearly labeled cold/warm conditions:

- time from activation to a focused launcher;
- search response latency while typing;
- application indexing duration and responsiveness during indexing;
- idle memory usage;
- action completion, timeout, cancellation, and failure rates; and
- clarity of failures for unavailable commands and unsupported desktop
  capabilities, assessed through focused usability checks.

Performance targets should be chosen after a baseline exists. The goal is a
responsive native launcher, not a misleading target achieved by omitting
indexing, errors, accessibility, or capability checks.

## 11. Deferred and alternative directions

### Stage Manager scope

Stage Manager initially means named workspace/application groups. It is not a
promise of universal window tiling, compositor control, or identical behavior
across desktops. A broader window-layout engine remains deferred until the
platform boundaries and failure semantics are proven.

### Tauri alternative

Tauri remains a reasonable alternative if rapid web-based visual iteration
becomes more important than a single native UI stack. It combines Rust with
HTML/CSS/JavaScript and system webviews, but introduces a frontend/backend IPC
boundary and additional runtime prerequisites. It is not the default for the
initial implementation because Scene is a system-integrated Linux application
where low overhead and native process/input integration matter. See the
[Tauri architecture documentation](https://tauri.app/start/) and [Tauri Linux
prerequisites](https://tauri.app/start/prerequisites/).

### Other deferred features

- Broad third-party integration distribution and permissions marketplace.
- Cross-desktop parity before KDE behavior is reliable.
- Silent privilege escalation or background system mutation.
- A permanent dashboard, sidebar, or full desktop-shell replacement.
- Universal Copilot-key guarantees across hardware and sessions.
- Workspace behavior that cannot be expressed or tested through a platform
  capability.

## 12. Explicit assumptions and decision record

- KDE Plasma is the first desktop environment.
- The first release proves the launcher core, not the entire product vision.
- Copilot-key support is best effort and always has a configurable fallback.
- Stage Manager initially means workspace/application-group switching.
- The initial implementation is Rust plus GTK4.
- Electron is not the recommended default because Scene is a system-integrated
  Linux application where low overhead and native process/input integration
  matter.
- `PRODUCT_PLAN.md` is the only product artifact created at this stage;
  implementation files come in later milestones.

Progress should be reported against the milestone checklists with a clear
distinction between implemented, verified, manually validated, unsupported,
and deferred behavior.
