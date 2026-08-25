# KRunner Parity Plan

This document defines what Scene must do to **match** KRunner, and where it
must **exceed** it. It expands [`PRODUCT_PLAN.md`](../PRODUCT_PLAN.md)
Milestone 6 ("KDE replacement foundation") into a concrete, checkable track.

Parity milestones are numbered `P1`–`P6` so they never collide with the
product milestones `M0`–`M8`. Each `P` milestone names the `M` milestone it
rides on. `P` milestones are not a separate project; they are the acceptance
bar applied to the product track.

## 1. How to read this document

A capability is **matched** when a user who relied on the KRunner equivalent
can stop using it without losing behaviour they depended on. That is a higher
bar than "a similar result appears in the list": ranking, latency, keyword
syntax, and failure behaviour are part of the capability.

Three states are used throughout:

| State | Meaning |
| --- | --- |
| **Match** | Scene must reproduce the behaviour a KRunner user depends on |
| **Exceed** | Scene must do something KRunner architecturally cannot |
| **Decline** | Scene deliberately will not reproduce it; the reason is stated |

"Decline" is not a backlog item. It is a decision, and each one is justified
below. A declined capability must still be *reported* honestly when a user
searches for it, rather than silently returning nothing.

## 2. Verified baseline

The inventory below was read from this development machine — Fedora 44,
Plasma 6.7.4, `plasma-workspace-6.7.4-1.fc44.x86_64` — by listing
`/usr/lib64/qt6/plugins/kf6/krunner/*.so` and
`/usr/share/krunner/dbusplugins/*.desktop`. It is what KRunner actually ships
here, not what upstream documents.

**31 runners: 25 compiled plugins and 6 D-Bus runners.**

Runner *descriptions* below are the established behaviour of these long-stable
KDE components; only the inventory itself is machine-verified. Anything that
turns out to differ in practice should be corrected here rather than worked
around in code.

### 2.1 Architectural findings

Three properties of KRunner shape this whole plan. All three were verified
directly.

**The UI cannot be restyled without recompiling.** In Plasma 5 the launcher
QML was an editable file under
`/usr/share/plasma/shells/…/contents/runcommand/`. In 6.7 it is compiled into
the binary as a QML module resource:

```
$ strings /usr/bin/krunner | grep qml
RunCommand 254.0 qml/RunCommand.qml
```

What remains user-configurable is the Plasma Style SVGs, the colour scheme,
fonts, and two keys in `krunnerrc` (`FreeFloating`, `RetainPriorSearch`).

**A runner cannot report a result.** The extension contract is:

```
org.kde.krunner1
  .Match    s  → a(sssida{sv})    query in, matches out
  .Run      ss → ·                 matchId, actionId → void
  .Actions  ·  → a(sss)
  .Config   ·  → a{sv}
  .Teardown ·  → ·
```

`Run` returns nothing. There is no channel for success, failure, output,
progress, or cancellation, and no way to interpose a confirmation step before
execution. KRunner is fire-and-forget by design.

**Consequently, three of Scene's product principles cannot be expressed as a
KRunner runner at all**: principle 4 (visible consequences), principle 5 (safe
by default / explicit confirmation), and the bounded-execution requirements of
`PRODUCT_PLAN.md` Milestone 3. This is the substantive reason Scene is a
separate application rather than a set of runners, and it is also where
"better than KRunner" is earned rather than asserted.

### 2.2 Runner inventory

#### Launching and finding things

| Runner | Capability | Scene disposition |
| --- | --- | --- |
| `krunner_services` | Installed applications from `.desktop` entries | Match — P1 |
| `kwin-runner-windows` *(D-Bus)* | Switch to an open window | Match — P3 |
| `plasma-runner-baloosearch` *(D-Bus)* | File search by name and content | Match — P4 |
| `krunner_recentdocuments` | Recently opened documents | Match — P4 |
| `krunner_placesrunner` | KDE Places / bookmarked folders | Match — P4 |
| `krunner_bookmarksrunner` | Browser bookmarks | Match — P4 |
| `plasma-runner-browsertabs` *(D-Bus)* | Open browser tabs | Decline — §6 |
| `plasma-runner-browserhistory` *(D-Bus)* | Browser history | Decline — §6 |
| `krunner_appstream` | Applications available to install | Match — P5 |
| `locations` | Open a path, URL, or `mailto:` | Match — P1 |
| `helprunner` | KDE documentation lookup | Decline — §6 |

#### Computing and converting

| Runner | Capability | Scene disposition |
| --- | --- | --- |
| `calculator` | Arithmetic, including `=` prefix | Match — P2 |
| `unitconverter` | Unit and currency conversion | Match — P2 |
| `org.kde.datetime` | Current date/time, timezone conversion | Match — P2 |
| `krunner_colors` | Colour code conversion and preview | Match — P2 |
| `krunner_charrunner` | Special character by code point | Match — P2 |
| `krunner_dictionary` | Word definitions | Decline — §6 |
| `krunner_spellcheck` | Spelling suggestions | Decline — §6 |

#### System, session, and settings

| Runner | Capability | Scene disposition |
| --- | --- | --- |
| `krunner_systemsettings` | Open a System Settings module | Match — P3 |
| `krunner_powerdevil` | Suspend, hibernate, brightness | Match + Exceed — P3 |
| `krunner_sessions` | Log out, switch user, lock | Match + Exceed — P3 |
| `krunner_kill` | Find and terminate a process | Match + Exceed — P3 |
| `krunner_keys` | Trigger a configured global shortcut | Match — P3 |
| `krunner_kwin` | KWin scripting actions | Decline — §6 |
| `krunner_plasma-desktop` | Plasma desktop actions | Decline — §6 |
| `plasma-runnners-activities` *(D-Bus)* | Switch KDE Activity | Match — P6 |

#### Commands and the web

| Runner | Capability | Scene disposition |
| --- | --- | --- |
| `krunner_shell` | Run an arbitrary shell command | **Redesign** — §5.4 |
| `krunner_webshortcuts` | Keyworded web search (`gg:`, `wp:`) | Match — P2 |

#### Application-specific

| Runner | Capability | Scene disposition |
| --- | --- | --- |
| `krunner_katesessions` | Open a Kate session | Decline — §6 |
| `krunner_konsoleprofiles` | Open a Konsole profile | Decline — §6 |
| `org.kde.neochat` *(D-Bus)* | Open a Matrix room | Decline — §6 |

## 3. Cross-cutting requirements

These apply to every provider and are not restated per milestone. A parity
milestone is not complete unless these hold for the providers it adds.

### 3.1 Performance

KRunner is fast and users notice regressions immediately. Targets are set
after a baseline exists (`PRODUCT_PLAN.md` §10), but the shape is fixed:

- [ ] Activation to a focused, typeable search field is not perceptibly slower
      than KRunner on the same machine, measured cold and warm. Scene's half is
      measured (`M8`, `docs/measurements.md`): 145-260 ms from activation to
      first frame for a resident instance, 379-485 ms for a cold process. The
      comparison is not, because KRunner is not resident in this session and
      starting it to time it would time a cold start.
- [ ] No provider blocks the UI thread. A slow provider delays only its own
      results.
- [ ] Results for a keystroke either arrive or are superseded; a stale query's
      results never overwrite a newer query's.
- [ ] Every provider has a timeout, and exceeding it is a visible state rather
      than a silently missing result group.
- [ ] Idle memory is measured and recorded; a resident launcher that costs
      more than KRunner needs a stated reason. Half met by `M8`: measured and
      recorded in `docs/measurements.md` — 108 MiB for a launcher that has
      presented once, and 424 MiB for the instance that had been resident and
      in use for five hours on this machine. Forty and four hundred synthetic
      activations do not reproduce that growth, so the reason is not yet
      stated, and the item stays open until it is.

### 3.2 Ranking

- [x] Ranking is deterministic: the same query against the same index gives
      the same order — and, since `M4.5`, against the same history too, which
      is an explicit argument to `search::search` rather than ambient state.
- [x] Provider priority is explicit and configurable, not an accident of
      registration order.
- [ ] Exact and prefix matches outrank fuzzy matches; title matches outrank
      keyword matches.
- [x] History and frequency adjust ranking only after the deterministic
      baseline is in place, and can be disabled. `search::History`, added in
      `M4.5`: bounded below the matcher's own field penalties so it can lift a
      result within its group but never overturn a title match or cross a
      group boundary, and switched off with `SCENE_HISTORY=off`. A settings
      surface for it is P6's.

### 3.3 Failure behaviour

- [x] A provider that throws, times out, or returns malformed data is
      isolated. Other result groups and the launcher shell keep working.
- [x] A failed provider is reported in the UI, not silently dropped.
- [x] A capability that is unavailable on this session says so, in words, when
      the user searches for it.

### 3.4 Accessibility and input

Inherited from `PRODUCT_PLAN.md` §4 and applied to every new result type:

- [ ] New result types carry correct accessible names, roles, and descriptions.
- [ ] Selected state is distinguishable without relying on hue.
- [ ] Every new action is reachable and executable by keyboard alone.
- [ ] Reduced-motion and high-contrast preferences are respected.

## 4. Parity milestones

### P1 — Application and location launching

**Rides on:** `M2` — Application discovery and launching
**Replaces:** `krunner_services`, `locations`
**Status:** complete except where noted, on Fedora 44 / Plasma 6.7 / Wayland.

**Outcome:** A user can find and launch any installed application, and open
any path or URL, at least as reliably as with KRunner.

- [x] Discover `.desktop` entries from all XDG data directories, honouring
      `NoDisplay`, `Hidden`, `OnlyShowIn` / `NotShowIn`, and `TryExec`.
- [x] Index asynchronously; the launcher is usable while indexing runs.
- [x] Watch the application directories and re-index on change, without a
      restart.
- [x] Match on name, generic name, comment, keywords, and executable name.
- [x] Render the real themed icon, with a deterministic fallback.
- [x] Launch through the desktop application model, not a bare `exec`, so
      startup notification and scope are correct.
- [x] Support a `.desktop` entry's declared additional actions.
- [x] Open paths, `file://`, `http(s)://`, and `mailto:` URIs.
- [x] Report a launch failure in the UI, distinguishing missing executable,
      permission denied, and non-zero exit.

Notes on the gap `M4.5` closed and one qualified tick:

**Additional actions** are exposed through the same typed inline-action model
as every other provider. `Ctrl+K` opens the selected result's action menu and
GIO launches the named desktop action without extracting a shell command.

**Launch failure detail** is complete as of `M4.5`. A missing executable and
a permission failure were already distinguished; a detached process's non-zero
exit was not observed at all, because nothing was watching it. It is watched
now: `actions::detached` holds the child for 400 ms (`actions::START_WATCH`)
and reports its real exit status if it dies inside that window, and the
launcher re-presents itself to show a failure that arrives after it closed.

The boundary that remains is stated rather than papered over. An application
launched through the desktop's own application model is not Scene's child and
has no exit status Scene can read, so `Outcome::Started` says the program was
handed over rather than claiming `Outcome::Succeeded`, and the built-in "What
Scene Reports" result says so in the UI. Polling `/proc` for the pid was
considered and rejected: a wrapper that forks and exits — which several
`.desktop` entries do — would be reported as a failure it is not, and a wrong
failure is worse than an honest silence.

**Indexing** is synchronous rather than threaded. Discovery measures 12-19 ms
warm and 27 ms cold for 292 entries, and runs at startup before any window
exists, so there is no interval during which a visible launcher is unusable. A
thread here would be machinery with nothing to do. The measurement is recorded
in `apps.rs` so a regression shows up as a changed comment rather than a
silent cost. Re-indexing after an install is driven by `AppInfoMonitor` and was
verified by installing an entry while Scene was running and watching it appear.

**Honouring the hidden flags** is GIO's `g_app_info_should_show`, not Scene's
own parsing. On this machine that filters 292 entries to 81; 210 of the 211
hidden carry `NoDisplay=true`, which accounts for the difference.

### P2 — Answers and keyword syntax

**Rides on:** `M3` — Safe command and integration framework
**Replaces:** `calculator`, `unitconverter`, `org.kde.datetime`,
`krunner_colors`, `krunner_charrunner`, `krunner_webshortcuts`

**Outcome:** Queries that have an answer produce the answer inline, and
keyworded queries reach the right provider.

- [x] A query-parsing layer that recognises provider keywords and prefixes
      without hard-coding them into the UI.
- [x] Arithmetic, including an explicit `=` prefix, with the result
      copyable to the clipboard.
- [x] Unit conversion across at least length, mass, temperature, area,
      volume, time, and data.
- [x] Current date and time, and conversion between named timezones.
- [x] Colour conversion between hex, RGB, and HSL, with a visible swatch that
      is also labelled in text.
- [x] Character lookup by Unicode code point and by name.
- [x] Web shortcuts with user-configurable keywords, seeded from the user's
      existing KDE web shortcuts where they can be read.
- [x] An answer result states its provider, so an unexpected answer is
      traceable.

### P3 — Session, system, and windows

**Rides on:** `M3` — Safe command and integration framework
**Replaces:** `krunner_systemsettings`, `krunner_powerdevil`,
`krunner_sessions`, `krunner_kill`, `krunner_keys`, `kwin-runner-windows`

**Outcome:** A user can reach system state and open windows from the
launcher, and destructive actions are confirmed rather than fired blind.

- [x] List and switch to open windows, showing application, title, and
      desktop/activity.
- [x] Open a named System Settings module.
- [x] Power actions: suspend, hibernate, lock, log out, reboot, shut down.
- [x] **Exceed:** every power and session action is confirmed, naming the
      consequence, before anything happens.
- [x] Find a process by name and terminate it.
- [x] **Exceed:** process termination is confirmed, naming the process and
      PID, and reports whether the signal actually took effect.
- [x] Trigger a configured KDE global shortcut by name.
- [x] All of the above degrade to a clear unsupported-capability result on a
      non-Plasma session rather than failing obscurely.

### P4 — Files, places, and history

**Rides on:** `M3` — Safe command and integration framework
**Replaces:** `plasma-runner-baloosearch`, `krunner_recentdocuments`,
`krunner_placesrunner`, `krunner_bookmarksrunner`

**Outcome:** A user can find their own files and folders as readily as their
applications.

- [x] File search by name, with content search where an index is available.
- [x] Use the existing Baloo index when it is present and enabled; report
      plainly when it is not, and do not silently build a competing index.
- [x] Recently opened documents.
- [x] KDE Places entries, including remote and removable entries, with
      unmounted targets marked as such.
- [x] Browser bookmarks from at least one Firefox-family and one
      Chromium-family profile, read-only, with the profile named in the UI.
- [x] File results offer open, open-containing-folder, and copy-path actions.
- [x] File-content search is off by default and its privacy implication is
      stated where it is enabled.

### P5 — Packages and installable applications

**Rides on:** `M4` — Distro capability adapters
**Replaces:** `krunner_appstream`

**Outcome:** A user can find an application they do not yet have, and install
it deliberately.

**Status:** complete except where noted, with adapter commands verified on
Fedora 44, `debian:stable-slim`, and `archlinux:latest`.

- [x] Search available packages through the detected package manager.
- [x] Show package metadata: version, size, repository, summary.
- [x] Distinguish installed from available in the result itself.
- [x] Offer install and remove **only** behind explicit confirmation naming
      the operation and target.
- [x] Escalate privilege through the desktop's supported mechanism, never by
      assuming `sudo`.
- [x] Report the real outcome, including permission failure, missing tool, and
      non-zero exit — never success merely because a process started.
- [x] The launcher's core behaviour is unaffected when no adapter is usable.

Notes on the merged result and on the shape of the rest:

**Installed versus available** is now one asynchronously assembled row.
Repository search, metadata, and local installed state run away from the UI
thread; the row labels itself Installed, Available, or Not found and exposes
the applicable inspect/install/remove operations through inline actions.

**Results are executed, not streamed.** A package query builds its command on
every keystroke but runs nothing until Enter. That keeps the launcher within
`PRODUCT_PLAN.md` principle 1 without a per-keystroke process, and it is what
lets the exact argument vector be shown before it runs — §5.4's requirement,
met here rather than deferred. The cost is that a package search is a
deliberate action rather than live-as-you-type; that trade is stated in the
migration note P6 owns.

**Privilege** is `pkexec` and nothing else. There is no `sudo` fallback and no
Scene-owned password prompt: without PolicyKit, install and remove report
themselves unavailable.

### P6 — Shell parity and daily-driver readiness

**Rides on:** `M6` — KDE replacement foundation
**Replaces:** `plasma-runnners-activities`; closes the remaining shell gaps

**Outcome:** Scene is what the user actually reaches for, and KRunner's
shortcut can be reassigned without regret.

- [x] Command history: recall, re-run, and clear, with history off-by-default
      or clearable in one action.
- [x] Per-provider enable/disable and reordering in a settings surface.
- [x] Inline actions on results, reachable by keyboard.
- [x] Switch KDE Activity.
- [x] Autostart integration so Scene is resident and warm at login.
- [ ] Single-instance activation stays correct under rapid repeated toggling.
- [x] A documented migration note covering what a KRunner user gains, loses,
      and must reconfigure.
- [ ] Manual daily-driver validation: KRunner unbound for one week, with the
      gaps encountered recorded in this document.

## 5. Where Scene must exceed KRunner

Parity alone does not justify a new application. These four are the reasons
Scene exists, and each is architecturally impossible as a KRunner runner.

### 5.1 Visible execution state

KRunner's `Run` returns `void`. Scene must show, for any action that is not
instantaneous: that it is running, its output, its exit status, and its
failure reason. This is `PRODUCT_PLAN.md` principle 4, and it is the single
largest functional difference.

### 5.2 Confirmation before consequence

KRunner executes the selected match immediately. Scene must interpose an
explicit confirmation for any action that changes durable system state,
naming the operation and its target, and must not allow keyboard ambiguity or
a stale selection to bypass it.

### 5.3 Bounded execution

Every Scene action declares a timeout, an output limit, and cancellation
behaviour. A long-running action can be cancelled from the launcher. KRunner
has nowhere to express any of this.

### 5.4 A safe replacement for `krunner_shell`

`krunner_shell` turns arbitrary query text into a shell command. Scene's
safety model (`PRODUCT_PLAN.md` §6) forbids exactly that, so this is a
redesign rather than a match or a decline:

- [x] Shell execution is never implicit. Typing text that happens to name a
      binary does not offer to run it as a shell command.
- [x] An explicit, clearly-marked command action exists, with a declared
      executable, arguments, environment, working directory, timeout, and
      output limit.
- [x] It shows the exact argument vector that will be executed, unexpanded,
      before running it.
- [x] It requires confirmation.
- [x] Its output, exit status, and failure reason are visible afterwards.

This is deliberately less convenient than `krunner_shell` for the user who
wants a one-keystroke shell. That trade is the product's position, and the
migration note in P6 must say so plainly rather than implying parity.

## 6. Declined capabilities

Each of these is a decision, not an omission. When a user's query clearly
targets one, Scene should say the capability is not provided rather than
return nothing.

| Capability | Reason |
| --- | --- |
| Browser tabs, browser history | Requires a live browser extension channel per browser family; ongoing maintenance cost is disproportionate to the benefit, and history search carries a privacy cost the launcher should not take on by default. Bookmarks (P4) cover the durable, read-only subset. |
| Dictionary, spellcheck | Better served by the tools already focused on them; no launcher-specific advantage. |
| KDE help documentation | Narrow audience; `locations` (P1) already opens help URLs. |
| KWin scripting actions, Plasma desktop actions | Deeply Plasma-shell-internal, with no stable cross-desktop contract. Reconsider only if a specific action proves load-bearing in P6 validation. |
| Kate sessions, Konsole profiles, Matrix rooms | Per-application integrations. The right long-term answer is the integration contract from `M3`, so third parties can add them — not built-in special cases. |

## 7. Proving parity

Parity is a claim about behaviour, so it needs evidence, not a checked box.

- [ ] Each `P` milestone has automated tests for its ranking, parsing, and
      failure paths, per `PRODUCT_PLAN.md` §9.
- [ ] A recorded comparison run: a fixed set of representative queries issued
      to both KRunner and Scene on the same machine, with results and latency
      captured side by side.
- [ ] Every **Decline** is confirmed still-correct at P6, or moved.
- [ ] Every **Exceed** has a test proving the KRunner-impossible behaviour
      actually happens — a confirmation that blocks, output that appears, a
      cancellation that takes effect.
- [ ] The daily-driver week in P6 is completed and its findings recorded here.

Progress is reported against these checklists using the same distinction the
product plan requires: implemented, verified, manually validated, unsupported,
and deferred are different states and must not be conflated.
