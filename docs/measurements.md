# What Scene costs

`PRODUCT_PLAN.md` §10 asks for measurements before targets, with cold and warm
conditions labelled. This is the recorded baseline, and nothing in it is a
target: targets come after a baseline exists, and this is the baseline.

Take your own with:

```sh
cargo build --release
./target/release/scene --measure
```

It prints what it observed on the machine that ran it and says what it could
not observe — Scene cannot see whether the page cache was cold, or whether this
is the first run since boot, so the conditions below are stated by hand.

## Conditions

| | |
| --- | --- |
| Machine | Fedora Linux 44, KDE Plasma 6.7, Wayland, GTK 4.22, 8 processors, 23 GiB |
| Binary | `cargo build --release`, Scene 0.1.0, 2026-08-24 |
| Installed applications | 292 desktop entries, 81 of them shown |
| Page cache | Warm. The machine had been up 31 hours and the binary had just been built. A first-run-after-boot figure is not recorded, because a reboot was not available |
| Other load | The development machine was **not idle**. Three unrelated containers were running throughout, and the readings are given at two load averages to show what that costs |

Every number below is from three consecutive runs at each load.

## Startup

| | load ≈ 21 | load ≈ 46 |
| --- | --- | --- |
| Process start to first frame | 379 – 485 ms | 456 – 831 ms |
| Activation to first frame | 145 – 260 ms | 194 – 414 ms |

The first figure is a cold *process*: GTK, the style sheet, the whole index,
and the first frame the compositor draws. The second is what a resident Scene
answers a global shortcut with, and is the one users feel — which is why the
autostart entry exists.

Both are sensitive to what else the machine is doing: the same build was 45%
slower to first frame with three busy containers alongside it. On an idle
machine the honest expectation is the left column or better.

## Indexing

| | Median of 3 runs |
| --- | --- |
| Desktop entries, first call in a process | 24 – 35 ms, 81 shown |
| Desktop entries, repeated | 21 – 29 ms |
| Every provider's index | 34 – 46 ms, 461 results |

Indexing runs before a window exists, and again only when the installed set
changes, so there is no visible surface to be unresponsive while it happens.

By provider, at load ≈ 21 — the breakdown `--measure` prints, which is what
makes a regression attributable:

```
applications              23.2 ms    81 results
recent-documents          7.19 ms   111 results
kde-places                2.97 ms    12 results
system-settings           1.38 ms    83 results
bookmarks                 1.22 ms   144 results
global-shortcuts         0.152 ms     0 results
packages                 0.095 ms     2 results
power-session            0.060 ms     9 results
… every remaining provider under 0.03 ms
```

### What this found

The first run of `--measure` ever taken reported **5590 ms** from process start
to first frame, and the per-provider breakdown put **5005 ms** of it in one
place: `bookmarks`. Firefox holds a write lock on `places.sqlite` for as long
as it runs, so a read-only connection waited out SQLite's five-second busy
timeout and then reported the database as locked — five seconds in front of
every start, for no bookmarks at all.

Opening it with `immutable=1` takes no lock: 144 bookmarks in 1.2 ms, and start
to first frame is now 0.4 s. The cost is stated rather than hidden — an
immutable read ignores the write-ahead log, so it sees the last checkpoint.

## Search latency

Over 200 runs per query, history off, at load ≈ 21. *Ranking* is
`search::search` over the 461-item index. *Keystroke* is what one typed
character really costs: the providers that answer a query, then ranking over
their answers and the index together.

| Query | Ranking, median | p95 | Keystroke, median | p95 | Results |
| --- | --- | --- | --- | --- | --- |
| (resting list) | 0.021 ms | 0.023 ms | 0.022 ms | 0.024 ms | 50 |
| `fi` | 0.422 ms | 0.506 ms | 4.44 ms | 5.55 ms | 303 |
| `settings` | 0.456 ms | 0.571 ms | 4.34 ms | 4.91 ms | 126 |
| `= 12 * 8` | 0.417 ms | 0.515 ms | 4.37 ms | 4.86 ms | 0 in the index; the answer is generated |
| `qzxjv` | 0.417 ms | 0.532 ms | 4.30 ms | 5.03 ms | 4 |

At load ≈ 46 the medians move little (0.44 – 0.51 ms ranking, 4.8 – 5.5 ms per
keystroke) but the p95 roughly doubles.

Two things are worth reading off this table. Ranking is not where the time
goes: about half a millisecond over 461 items, and the resting list — which
matches everything and ranks nothing — is twenty times cheaper again. The
**4 ms** is the providers answering, plus the configuration read that
`answers` does on every keystroke. That is comfortably inside a 60 Hz frame, so
typing is not visibly affected, but it is where to look first if it ever is.

## Memory

| | |
| --- | --- |
| Resident set, launcher built and presented | 108 – 109 MiB |
| After 40 more activations | 120 – 125 MiB (+11 to +16 MiB) |
| After 400 more activations | 89.8 MiB (−18.8 MiB) |

**Repeated activation does not grow the process.** The rise at 40 activations
does not continue: by 400 the process is *smaller* than it started, which is
caches warming and an allocator reusing its own freed pages rather than a leak.

**But a real resident instance does grow, and this measurement does not
reproduce it.** The instance running on this machine — started 5.1 hours
earlier, used normally — held **424 MiB**, of which 414 MiB was anonymous
private memory (268 MiB heap). Something in ordinary use accumulates what
40 or 400 synthetic activations do not.

That is recorded rather than explained. The untested suspect is the
asynchronous path: `answers_async` spawns a future per keystroke for the D-Bus
runners, and Baloo, window, Activity and currency answers only run with a main
loop turning, which the synthetic loop never exercises with real queries. The
next measurement to take is a resident instance driven with real typing, and it
belongs with the field trial rather than here.

Until then, `docs/krunner-parity.md` §3.1's *"idle memory is measured and
recorded; a resident launcher that costs more than KRunner needs a stated
reason"* is **half met**: it is measured and recorded, and the reason is not
yet stated. No comparison with KRunner was taken, because KRunner is not
resident in this session and starting it to measure it would have measured a
cold start rather than a day's use.

## Not measured yet

`PRODUCT_PLAN.md` §10 also asks for action completion, timeout, cancellation
and failure rates, and for usability checks on how clear failures are. Those
need real use over time rather than a benchmark, so they belong to Milestone
6's field trial. The clarity of a failure is tested — every `Outcome` states
itself in words, and the suite proves it — but how often each one happens in a
week of real use is not something this machine has been asked yet.
