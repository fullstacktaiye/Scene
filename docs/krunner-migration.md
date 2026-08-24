# Moving from KRunner to Scene

Scene's Milestone 6 implementation is ready for a one-week daily-driver trial.
Do not remove KRunner; temporarily unbind its shortcut so reverting is one
settings change.

## What to reconfigure

1. Run `./scripts/install-user.sh` so the desktop launches the current build.
2. Open **Scene Settings** with `Ctrl+,`, enable **Start Scene at login**, and
   review provider order and privacy switches.
3. In KDE System Settings, move the shortcut normally used for KRunner to
   Scene. Scene reports the active KDE binding but deliberately does not edit
   KDE's shortcut configuration itself.
4. Leave command history and file-content search off unless their persistence
   and privacy trade-offs are wanted.

## What changes

Scene adds typed, confirmable actions, visible output and failures, provider
ordering, result actions through `Ctrl+K`, and clear unavailable states. Its
explicit command syntax is `run PROGRAM ARGUMENT…`; it parses an argument
vector without invoking a shell, shows that vector and its bounds, then asks
for confirmation.

That safety model intentionally loses shell syntax: pipelines, redirection,
globbing, substitutions, aliases, and implicit execution are not expanded.
Scene also declines browser tabs/history, dictionary and spellcheck, KDE help,
Plasma-internal actions, and application-specific session/room runners. Search
for those concepts produces a result explaining the decision.

## Field-trial record

For seven days, record missing results, slower workflows, incorrect ranking,
provider failures, activation issues, and idle-memory/latency observations in
[`krunner-parity.md`](krunner-parity.md). Milestone 6 is accepted only after
that record is complete; code completion alone does not close the manual gate.
