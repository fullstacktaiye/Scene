//! `scene --measure`: startup, indexing, search-latency and idle-memory
//! numbers, taken on the machine that runs it.
//!
//! The product plan asks for measurements before targets, and for cold and
//! warm conditions to be labelled. Two of those labels are not Scene's to
//! give: it cannot see the page cache, and it cannot know whether this is the
//! first run since boot. So this prints what it observed, names the machine
//! and session it observed it on, and leaves the condition to whoever records
//! the run in `docs/measurements.md`.
//!
//! It is a development tool. Nothing here runs unless the flag names it, and
//! the launcher's behaviour does not depend on it.

use std::time::{Duration, Instant};

use gtk::glib;
use gtk::prelude::*;

use crate::platform::DesktopSupport;
use crate::search::{self, History, Item};
use crate::{apps, integrations, ui};

/// The queries latency is measured over: the resting list, a prefix that
/// matches many applications, a word that matches a few, an answer that only
/// exists while the query asks for it, and one that matches nothing.
const QUERIES: [&str; 5] = ["", "fi", "settings", "= 12 * 8", "qzxjv"];

/// Enough runs for a median and a 95th percentile to mean something, and few
/// enough that the whole report is a couple of seconds.
const RUNS: usize = 200;

/// Indexing is fast enough that one reading is mostly noise.
const INDEX_RUNS: usize = 10;

/// How many times the launcher is shown and hidden again before its memory is
/// read a second time. A resident Scene is activated dozens of times a day, so
/// what one activation costs and keeps matters more than what one start does.
const ACTIVATIONS: usize = 40;

pub fn run(app: &gtk::Application, started: Instant) {
    let launcher = ui::Launcher::build(app);
    let activation = Instant::now();
    launcher.activate();

    let app = app.clone();
    let repeat = launcher.clone();
    launcher.on_first_frame(move || {
        let presented = Instant::now();
        report(started, activation, presented);

        // One activation per main-loop turn, so GTK really does the work of
        // presenting and hiding rather than collapsing it all into one frame.
        let launcher = repeat.clone();
        let app = app.clone();
        let settled = resident_set();
        let mut left = ACTIVATIONS * 2;
        glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
            if left > 0 {
                launcher.activate();
                left -= 1;
                return glib::ControlFlow::Continue;
            }
            match (settled, resident_set()) {
                (Some(before), Some(after)) => println!(
                    "  after {ACTIVATIONS} more activations     {:.1} MiB   {:+.1} MiB",
                    after as f64 / 1024.0,
                    (after as f64 - before as f64) / 1024.0
                ),
                _ => println!("  after {ACTIVATIONS} more activations     unavailable"),
            }
            app.quit();
            glib::ControlFlow::Break
        });
    });
}

fn report(started: Instant, activation: Instant, presented: Instant) {
    println!("Scene {} measurements", env!("CARGO_PKG_VERSION"));
    println!("  session       {}", DesktopSupport::detect().summary());
    println!("  machine       {}", machine());
    println!("  binary        {}", binary());
    println!(
        "  condition     as run; whether the page cache was cold is not\n\
         \x20               something Scene can observe, so the run records it"
    );

    println!();
    println!("Startup");
    println!(
        "  process start to first frame       {}",
        milliseconds(presented.duration_since(started))
    );
    println!(
        "  activation to first frame          {}",
        milliseconds(presented.duration_since(activation))
    );
    println!(
        "  (the first is a cold process: it includes GTK, the style sheet,\n\
         \x20  indexing and the first frame. A resident Scene answers a global\n\
         \x20  shortcut with the second.)"
    );

    println!();
    println!("Indexing");
    let (first, applications) = timed(apps::installed);
    let repeats = repeat(INDEX_RUNS, || {
        apps::installed();
    });
    println!(
        "  desktop entries, first in-process  {}   {} shown",
        milliseconds(first),
        applications.len()
    );
    println!(
        "  desktop entries, repeated          {}   median of {INDEX_RUNS}",
        milliseconds(median(&repeats))
    );
    let (built, items) = timed(search::index);
    println!(
        "  every provider's index             {}   {} results",
        milliseconds(built),
        items.len()
    );
    println!();
    println!("  by provider, slowest first");
    let mut providers = integrations::index_by_provider();
    providers.sort_by_key(|(_, elapsed, _)| std::cmp::Reverse(*elapsed));
    for (metadata, elapsed, results) in providers {
        println!(
            "    {:<22} {:>10}   {results} results",
            metadata.id,
            milliseconds(elapsed)
        );
    }
    println!();
    println!(
        "  (indexing runs before a window exists, and again only when the\n\
         \x20  installed set changes, so there is no visible surface to be\n\
         \x20  unresponsive while it happens.)"
    );

    println!();
    println!("Search latency, over {RUNS} runs, history off");
    println!("  query                              median      p95   results");
    let history = History::disabled();
    for query in QUERIES {
        let ranked = search::search(query, &items, &history).len();
        let ranking = repeat(RUNS, || {
            search::search(query, &items, &history);
        });
        // What one keystroke really costs: the providers that answer a query
        // run on the GTK thread, and then everything is ranked together.
        let keystroke = repeat(RUNS.min(50), || {
            let answers = integrations::answers(query);
            let all: Vec<&Item> = items.iter().chain(answers.iter()).collect();
            search::search(query, &all, &history);
        });
        println!(
            "  ranking  {:<24}  {:>8} {:>8}   {ranked}",
            display(query),
            milliseconds(median(&ranking)),
            milliseconds(percentile(&ranking, 95)),
        );
        println!(
            "  keystroke{:<24}  {:>8} {:>8}",
            "",
            milliseconds(median(&keystroke)),
            milliseconds(percentile(&keystroke, 95)),
        );
    }

    println!();
    println!("Memory");
    match resident_set() {
        Some(kilobytes) => println!(
            "  resident set, launcher presented   {:.1} MiB",
            kilobytes as f64 / 1024.0
        ),
        None => println!("  resident set                       unavailable (/proc/self/status)"),
    }
    println!(
        "  (this process holds one built launcher, the index above, and the\n\
         \x20  window it presented. A resident `scene --background` instance at\n\
         \x20  rest is the same thing with its window hidden.)"
    );
}

fn timed<T>(work: impl FnOnce() -> T) -> (Duration, T) {
    let start = Instant::now();
    let value = work();
    (start.elapsed(), value)
}

fn repeat(runs: usize, mut work: impl FnMut()) -> Vec<Duration> {
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let start = Instant::now();
        work();
        samples.push(start.elapsed());
    }
    samples.sort_unstable();
    samples
}

/// `samples` is already sorted by [`repeat`].
fn median(samples: &[Duration]) -> Duration {
    percentile(samples, 50)
}

fn percentile(samples: &[Duration], percent: usize) -> Duration {
    if samples.is_empty() {
        return Duration::ZERO;
    }
    let index = (samples.len() * percent / 100).min(samples.len() - 1);
    samples[index]
}

fn milliseconds(duration: Duration) -> String {
    let milliseconds = duration.as_secs_f64() * 1000.0;
    if milliseconds >= 10.0 {
        format!("{milliseconds:.1} ms")
    } else if milliseconds >= 1.0 {
        format!("{milliseconds:.2} ms")
    } else {
        format!("{milliseconds:.3} ms")
    }
}

fn display(query: &str) -> String {
    if query.is_empty() {
        String::from("(resting list)")
    } else {
        format!("{query:?}")
    }
}

fn machine() -> String {
    let name = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|line| line.strip_prefix("PRETTY_NAME=").map(str::to_string))
        })
        .map(|value| value.trim_matches('"').to_string())
        .unwrap_or_else(|| String::from("unknown distribution"));
    let processors = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(0);
    format!("{name}, {processors} processors")
}

fn binary() -> String {
    std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| String::from("unknown"))
}

/// The kernel's own figure for this process, in kilobytes.
fn resident_set() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status.lines().find_map(|line| {
                line.strip_prefix("VmRSS:")?
                    .split_whitespace()
                    .next()?
                    .parse()
                    .ok()
            })
        })
}
