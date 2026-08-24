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

- [ ] `PRODUCT_PLAN.md` explains what Scene is and is not.
- [ ] Product principles, initial user journeys, and visual direction are
      documented.
- [ ] Keyboard, accessibility, integration, safety, and KDE-first assumptions
      are explicit.
- [ ] The full vision and deferred features are separated from the first
      implementation scope.
- [ ] Every later milestone has an outcome and completion checklist.

### Milestone 1 — Launcher shell

**User-visible outcome:** A local GTK4 application opens a centered launcher,
focuses search, accepts queries, navigates results, executes static in-memory
results, and closes reliably.

**Checklist:**

- [ ] Centered launcher window with focused search field.
- [ ] Static in-memory result provider.
- [ ] Query updates, Up/Down navigation, Enter, Escape, and empty-result state.
- [ ] Repeated activation has no duplicate windows or stale state.
- [ ] UI styling can evolve without changing search or action logic.
- [ ] Keyboard-only smoke path is reliable.

### Milestone 2 — Application discovery and launching

**User-visible outcome:** Scene finds installed graphical applications and
launches them through the desktop application model.

**Checklist:**

- [ ] Discover Linux desktop entries asynchronously or incrementally.
- [ ] Render application icons, names, descriptions, and categories where
      available.
- [ ] Add deterministic fuzzy search and ranking.
- [ ] Present launch failures in the UI.
- [ ] Add recent/frequent ranking only after baseline ranking is deterministic.
- [ ] Include a valid `.desktop` entry for Scene itself and follow the
      [desktop-entry specification](https://specifications.freedesktop.org/desktop-entry-spec/latest/).
- [ ] Check packaging behavior against [Fedora desktop application packaging
      guidance](https://docs.fedoraproject.org/nn/packaging-guidelines/).

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

- [ ] Implement shared adapters for Debian/Ubuntu-family, Fedora, and Arch.
- [ ] Expose package search, metadata, installed-package, and update
      capabilities where supported.
- [ ] Gate optional mutation actions behind confirmation.
- [ ] Detect executable capabilities rather than relying only on distro name.
- [ ] Report missing tools, permission issues, and unsupported operations
      clearly.
- [ ] Verify at least one Debian/Ubuntu-family, Fedora, and Arch environment.
- [ ] Verify core launcher behavior when an adapter is unavailable.

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
