//! Query matching and ranking over an in-memory result set.
//!
//! Ranking is deterministic: the same query, against the same items, with the
//! same [`History`], always produces the same order. Those three are the only
//! inputs. Providers hand this module typed items; it never executes anything
//! and never touches the UI.

use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use gtk::gio;
use gtk::prelude::*;

use crate::actions::Action;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// An answer to a query that named a package. These exist only while the
    /// query asks for them, which is why they lead.
    Package,
    Application,
    Folder,
    Web,
    Scene,
}

impl Kind {
    /// Groups appear in this order, best first.
    #[cfg(test)]
    fn priority(self) -> u8 {
        match self {
            Kind::Package => 0,
            Kind::Application => 1,
            Kind::Folder => 2,
            Kind::Web => 3,
            Kind::Scene => 4,
        }
    }

    pub fn tag(self) -> &'static str {
        match self {
            Kind::Package => "Package",
            Kind::Application => "Application",
            Kind::Folder => "Folder",
            Kind::Web => "Link",
            Kind::Scene => "Command",
        }
    }

    /// CSS class for the icon tile, and the icon used when the theme has no
    /// icon of its own for an item.
    pub fn slug(self) -> &'static str {
        match self {
            Kind::Package => "package",
            Kind::Application => "application",
            Kind::Folder => "folder",
            Kind::Web => "web",
            Kind::Scene => "scene",
        }
    }

    /// How many of this group to show when there is no query.
    ///
    /// Applications are the one group whose size is not Scene's to choose:
    /// they come from discovery, and there are 81 of them on the development
    /// machine. Filling the launcher with all of them before the user has
    /// typed anything buries the groups underneath and says nothing useful.
    /// Every other group is a small fixed catalogue Scene itself owns, so it
    /// shows in full. Typing searches all of them either way.
    /// The same limit, reachable from `ui`'s smoke harness.
    #[cfg(test)]
    pub(crate) fn resting_limit_for_tests(self) -> usize {
        self.resting_limit().unwrap_or(usize::MAX)
    }

    fn resting_limit(self) -> Option<usize> {
        match self {
            Kind::Application => Some(5),
            Kind::Package | Kind::Folder | Kind::Web | Kind::Scene => None,
        }
    }

    pub fn fallback_icon(self) -> &'static str {
        match self {
            Kind::Package => "package-x-generic-symbolic",
            Kind::Application => "application-x-executable-symbolic",
            Kind::Folder => "folder-symbolic",
            Kind::Web => "web-browser-symbolic",
            Kind::Scene => "starred-symbolic",
        }
    }
}

#[derive(Clone)]
pub struct Item {
    pub id: String,
    pub provider: String,
    pub provider_title: String,
    pub provider_priority: u16,
    pub title: String,
    pub subtitle: String,
    pub kind: Kind,
    /// The desktop's own icon, when it has one. Falls back to the kind's.
    pub icon: Option<gio::Icon>,
    /// A more specific label than the kind's own. Applications use their
    /// freedesktop category, so a browser reads "Network" rather than
    /// "Application".
    pub category: Option<String>,
    pub keywords: Vec<String>,
    pub action: Action,
    pub secondary_actions: Vec<ItemAction>,
}

#[derive(Clone, Debug)]
pub struct ItemAction {
    pub id: String,
    pub label: String,
    pub action: Action,
}

impl Item {
    /// The short label shown at the end of the row.
    pub fn tag(&self) -> &str {
        self.category.as_deref().unwrap_or_else(|| self.kind.tag())
    }
}

/// Everything Scene can currently find: the installed applications, plus the
/// built-in folders, links and commands.
pub fn index() -> Vec<Item> {
    crate::integrations::index()
}

/// Returns the indices of `items` that match `query`, grouped by kind and
/// ranked within each group. An empty query matches everything.
///
/// It takes anything that borrows an `Item`, so the caller can rank one owned
/// index and a set of query answers together without copying either.
///
/// `history` adjusts the order within a group and can never reorder the groups
/// themselves. Pass [`History::disabled`] for the deterministic baseline alone.
pub fn search<I: Borrow<Item>>(query: &str, items: &[I], history: &History) -> Vec<usize> {
    let needle = query.trim().to_lowercase();

    let mut hits: Vec<(u16, i32, usize)> = items
        .iter()
        .map(Borrow::borrow)
        .enumerate()
        .filter_map(|(i, item)| {
            let score = if needle.is_empty() {
                0
            } else {
                score(&needle, item)?
            };
            Some((
                item.provider_priority,
                -(score + history.bonus(&item.id)),
                i,
            ))
        })
        .collect();

    // Group, then best score, then the order the provider listed them in.
    hits.sort_unstable();
    if needle.is_empty() {
        hits = resting(hits, items);
    }
    hits.into_iter().map(|(_, _, i)| i).collect()
}

/// Trim each group to its [`Kind::resting_limit`].
///
/// This applies only with no query, where every item "matches" and the score
/// carries no information: what leads a group there is what the user has
/// actually used. A query ranks against something, so it is never trimmed —
/// searching still reaches every result.
fn resting<I: Borrow<Item>>(hits: Vec<(u16, i32, usize)>, items: &[I]) -> Vec<(u16, i32, usize)> {
    let mut kept = Vec::with_capacity(hits.len());
    let mut taken = BTreeMap::<String, usize>::new();

    for hit in hits {
        let item = items[hit.2].borrow();
        let count = taken.entry(item.provider.clone()).or_default();
        *count += 1;
        if item_resting_limit(item).is_none_or(|limit| *count <= limit) {
            kept.push(hit);
        }
    }
    kept
}

fn item_resting_limit(item: &Item) -> Option<usize> {
    match item.provider.as_str() {
        "applications" => Some(5),
        "bookmarks" | "recent-documents" | "kde-places" | "system-settings"
        | "global-shortcuts" => Some(5),
        "declined" => Some(0),
        _ => item.kind.resting_limit(),
    }
}

/// Best score across an item's searchable text, or `None` if nothing matches.
///
/// The penalties order the three fields against each other: a title hit always
/// beats a keyword hit, which always beats a hit in the description.
fn score(needle: &str, item: &Item) -> Option<i32> {
    let title = fuzzy(needle, &item.title.to_lowercase());
    let keywords = item
        .keywords
        .iter()
        .filter_map(|k| fuzzy(needle, &k.to_lowercase()))
        .max()
        .map(|s| s - 40);
    let subtitle = fuzzy(needle, &item.subtitle.to_lowercase()).map(|s| s - 80);

    title.max(keywords).max(subtitle)
}

/// Subsequence match. Rewards matches at word starts and runs of adjacent
/// characters, penalises the gaps it has to skip.
fn fuzzy(needle: &str, haystack: &str) -> Option<i32> {
    let hay: Vec<char> = haystack.chars().collect();
    let mut score = 0;
    let mut from = 0;
    let mut previous: Option<usize> = None;

    for want in needle.chars() {
        let at = from + hay[from..].iter().position(|&c| c == want)?;

        if previous == Some(at.wrapping_sub(1)) {
            score += 14; // adjacent to the last match
        }
        if at == 0 {
            score += 20; // start of the text
        } else if !hay[at - 1].is_alphanumeric() {
            score += 12; // start of a word
        }
        score -= ((at - from) as i32).min(8); // skipped characters

        previous = Some(at);
        from = at + 1;
    }

    // Prefer the shorter of two otherwise equal matches.
    Some(score - hay.len() as i32 / 8)
}

/// The recent and frequent adjustment to the deterministic baseline.
///
/// This is the one part of ranking that depends on what the user has done
/// before, so it is an explicit argument to [`search`] rather than ambient
/// state: the same query, items and history always produce the same order.
///
/// The adjustment is bounded on purpose. The largest bonus any item can carry
/// is [`MAX_FREQUENCY_BONUS`] + [`MAX_RECENCY_BONUS`] = 39, which stays below
/// the 40-point gap [`score`] puts between a title hit and a keyword hit. Use
/// can therefore lift a result within its group; it can never make a keyword
/// match outrank a title match, and it never crosses a group boundary.
#[derive(Debug, Default)]
pub struct History {
    enabled: bool,
    /// Where the history is persisted, when it is. `None` while disabled and
    /// in tests, which never touch the user's state directory.
    path: Option<PathBuf>,
    /// Captured when the history is loaded and moved only by [`record`], so
    /// the order does not drift under the user while they are typing.
    ///
    /// [`record`]: History::record
    now: u64,
    entries: BTreeMap<String, Use>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Use {
    count: u32,
    /// Seconds since the Unix epoch.
    last: u64,
}

/// The first line of the state file. A file that does not start with exactly
/// this is a history Scene cannot read, which is no history rather than an
/// error — and it is where a later format change announces itself.
const HISTORY_FORMAT: &str = "scene-history 1";
/// Enough to cover what one person actually launches, and bounded so the file
/// cannot grow without limit.
const HISTORY_LIMIT: usize = 512;
const MAX_FREQUENCY_BONUS: i32 = 24;
const MAX_RECENCY_BONUS: i32 = 15;

impl History {
    /// The deterministic baseline with no adjustment at all. Ranking tests use
    /// it, and so does a session where the user turned history off.
    pub fn disabled() -> Self {
        Self::default()
    }

    /// The user's history, or a disabled one when it is switched off or cannot
    /// be read. A history that fails to load is never an error the user has to
    /// deal with: the launcher ranks on the baseline and starts recording
    /// again.
    pub fn load_enabled(enabled: bool) -> Self {
        if !enabled {
            return Self::disabled();
        }
        let path = state_path();
        let text = path
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .unwrap_or_default();
        let mut history = Self::parse(&text, seconds_since_epoch());
        history.path = path;
        history
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        self.path = enabled.then(state_path).flatten();
        if enabled {
            self.save();
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.save();
    }

    /// Note that a result was chosen, and persist it.
    pub fn record(&mut self, id: &str) {
        if !self.enabled || id.is_empty() {
            return;
        }
        self.now = seconds_since_epoch();
        let used = self.entries.entry(id.to_string()).or_default();
        used.count = used.count.saturating_add(1);
        used.last = self.now;
        self.forget_oldest();
        self.save();
    }

    /// The bonus for one item, which is zero for anything never chosen.
    fn bonus(&self, id: &str) -> i32 {
        if !self.enabled {
            return 0;
        }
        let Some(used) = self.entries.get(id) else {
            return 0;
        };
        frequency_bonus(used.count) + self.recency_bonus(used.last)
    }

    /// Recency is bucketed rather than continuous, so two results used in the
    /// same session rank by how often they were used rather than by seconds.
    fn recency_bonus(&self, last: u64) -> i32 {
        const HOUR: u64 = 60 * 60;
        const DAY: u64 = 24 * HOUR;
        match self.now.saturating_sub(last) {
            age if age < HOUR => MAX_RECENCY_BONUS,
            age if age < DAY => 10,
            age if age < 7 * DAY => 5,
            age if age < 30 * DAY => 2,
            _ => 0,
        }
    }

    fn forget_oldest(&mut self) {
        while self.entries.len() > HISTORY_LIMIT {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(id, used)| (used.last, (*id).clone()))
                .map(|(id, _)| id.clone())
            else {
                return;
            };
            self.entries.remove(&oldest);
        }
    }

    fn parse(text: &str, now: u64) -> Self {
        let mut history = Self {
            enabled: true,
            path: None,
            now,
            entries: BTreeMap::new(),
        };
        let mut lines = text.lines();
        if lines.next().map(str::trim) != Some(HISTORY_FORMAT) {
            return history;
        }
        for line in lines {
            let mut fields = line.trim().splitn(3, ' ');
            let (Some(count), Some(last), Some(id)) = (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            let (Ok(count), Ok(last)) = (count.parse::<u32>(), last.parse::<u64>()) else {
                continue;
            };
            if !id.is_empty() {
                history.entries.insert(id.to_string(), Use { count, last });
            }
        }
        history
    }

    fn render(&self) -> String {
        let mut text = String::from(HISTORY_FORMAT);
        text.push('\n');
        for (id, used) in &self.entries {
            text.push_str(&format!("{} {} {id}\n", used.count, used.last));
        }
        text
    }

    /// Best effort. A state directory that cannot be written costs the user a
    /// better order next time, and nothing else.
    fn save(&self) {
        let Some(path) = self.path.as_ref() else {
            return;
        };
        if let Some(directory) = path.parent() {
            let _ = std::fs::create_dir_all(directory);
        }
        let _ = std::fs::write(path, self.render());
    }
}

fn frequency_bonus(count: u32) -> i32 {
    const STEPS: i32 = 8;
    (count.min(STEPS as u32) as i32) * (MAX_FREQUENCY_BONUS / STEPS)
}

/// `SCENE_HISTORY=off` turns the adjustment off entirely, which is the
/// "can be disabled" the ranking rules require. Milestone 6 gives it a
/// settings surface; until then it is one environment variable, like
/// `SCENE_DIRECTORY`.
/// The decision on its own, so it can be tested without touching the process
/// environment other tests are reading at the same time.
#[cfg(test)]
fn enabled_by(value: Option<&str>) -> bool {
    match value {
        Some(value) => !matches!(
            value.trim().to_lowercase().as_str(),
            "off" | "0" | "no" | "false"
        ),
        None => true,
    }
}

/// `$XDG_STATE_HOME/scene/history`, which is where state that survives a
/// restart but is not configuration belongs.
fn state_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("state"))
        })?;
    Some(base.join("scene").join("history"))
}

fn seconds_since_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default()
}

pub fn themed(name: &str) -> Option<gio::Icon> {
    Some(gio::ThemedIcon::new(name).upcast())
}

fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|w| w.to_string()).collect()
}

/// The built-in results: places worth opening and things Scene can say about
/// itself. Applications come from `apps::installed` instead.
pub fn catalogue() -> Vec<Item> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());

    let folder = |id: &str, title: &str, path: String, keywords: &[&str]| Item {
        id: id.to_string(),
        provider: "places".into(),
        provider_title: "Places".into(),
        provider_priority: 30,
        title: title.to_string(),
        subtitle: path.clone(),
        kind: Kind::Folder,
        icon: themed("folder"),
        category: None,
        keywords: words(keywords),
        action: Action::Open { target: path },
        secondary_actions: Vec::new(),
    };

    let link = |id: &str, title: &str, url: &str, keywords: &[&str]| Item {
        id: id.to_string(),
        provider: "web".into(),
        provider_title: "Web".into(),
        provider_priority: 40,
        title: title.to_string(),
        subtitle: url.to_string(),
        kind: Kind::Web,
        icon: themed("web-browser"),
        category: None,
        keywords: words(keywords),
        action: Action::Open {
            target: url.to_string(),
        },
        secondary_actions: Vec::new(),
    };

    vec![
        folder("dir.home", "Home", home.clone(), &["~", "user"]),
        folder(
            "dir.downloads",
            "Downloads",
            format!("{home}/Downloads"),
            &["saved", "files"],
        ),
        folder(
            "dir.documents",
            "Documents",
            format!("{home}/Documents"),
            &["docs", "papers"],
        ),
        link(
            "web.gtk",
            "GTK 4 Documentation",
            "https://docs.gtk.org/gtk4/",
            &["gtk", "docs", "reference", "toolkit"],
        ),
        link(
            "web.rust",
            "Rust Standard Library",
            "https://doc.rust-lang.org/std/",
            &["rust", "docs", "std", "reference"],
        ),
        Item {
            id: "scene.settings".to_string(),
            provider: "scene".into(),
            provider_title: "Scene".into(),
            provider_priority: 90,
            title: "Scene Settings".into(),
            subtitle: "Global shortcut and Copilot-key status".into(),
            kind: Kind::Scene,
            icon: themed("preferences-system-symbolic"),
            category: None,
            keywords: words(&[
                "settings",
                "shortcut",
                "hotkey",
                "copilot",
                "meta space",
                "configure",
            ]),
            action: Action::ShowSettings,
            secondary_actions: Vec::new(),
        },
        Item {
            id: "scene.about".to_string(),
            provider: "scene".into(),
            provider_title: "Scene".into(),
            provider_priority: 90,
            title: "About Scene".into(),
            subtitle: "What this build can do".into(),
            kind: Kind::Scene,
            icon: themed("help-about"),
            category: None,
            keywords: words(&["version", "help", "info"]),
            action: Action::Message {
                text: concat!(
                    "Scene ",
                    env!("CARGO_PKG_VERSION"),
                    " — Milestone 5. Global activation and honest Copilot-key status."
                )
                .into(),
            },
            secondary_actions: Vec::new(),
        },
        Item {
            id: "scene.reporting".to_string(),
            provider: "scene".into(),
            provider_title: "Scene".into(),
            provider_priority: 90,
            title: "What Scene Reports".into(),
            subtitle: "Which outcomes Scene actually watched".into(),
            kind: Kind::Scene,
            icon: themed("dialog-information"),
            category: None,
            keywords: words(&["launch", "failure", "outcome", "exit", "watch", "limits"]),
            action: Action::Message {
                text: format!(
                    "Scene watches a program it starts for {:.1} seconds and reports the exit status if it fails in that window. An installed application is handed to the desktop with its activation token: Scene has no handle on it after that, so it reports that it started rather than that it succeeded, and a failure later is not observed. Recent and frequent results rank higher; set SCENE_HISTORY=off to turn that off.",
                    crate::actions::START_WATCH.as_secs_f32()
                ),
            },
            secondary_actions: Vec::new(),
        },
        Item {
            id: "scene.keys".to_string(),
            provider: "scene".into(),
            provider_title: "Scene".into(),
            provider_priority: 90,
            title: "Keyboard Shortcuts".into(),
            subtitle: "How to drive Scene without a mouse".into(),
            kind: Kind::Scene,
            icon: themed("input-keyboard"),
            category: None,
            keywords: words(&["keys", "bindings", "navigation", "help"]),
            action: Action::Message {
                text: "Up and Down to move, Enter to open, Escape to clear then close, Ctrl+, for settings, Ctrl+Q to quit.".into(),
            },
            secondary_actions: Vec::new(),
        },
        Item {
            id: "scene.quit".to_string(),
            provider: "scene".into(),
            provider_title: "Scene".into(),
            provider_priority: 90,
            title: "Quit Scene".into(),
            subtitle: "Stop the launcher entirely".into(),
            kind: Kind::Scene,
            icon: themed("application-exit"),
            category: None,
            keywords: words(&["exit", "close", "stop"]),
            action: Action::Quit,
            secondary_actions: Vec::new(),
        },
    ]
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A fixed set of items, so ranking tests do not depend on the machine's
    /// installed applications or on the catalogue's current contents. The UI
    /// smoke harness drives the launcher against it for the same reason.
    pub(crate) fn fixture() -> Vec<Item> {
        let item = |id: &str, title: &str, subtitle: &str, kind: Kind, keywords: &[&str]| Item {
            id: id.to_string(),
            provider: if kind == Kind::Application {
                "applications".into()
            } else {
                kind.slug().into()
            },
            provider_title: kind.tag().into(),
            provider_priority: u16::from(kind.priority()),
            title: title.to_string(),
            subtitle: subtitle.to_string(),
            kind,
            icon: None,
            category: None,
            keywords: words(keywords),
            action: Action::Message {
                text: title.to_string(),
            },
            secondary_actions: Vec::new(),
        };

        vec![
            item(
                "firefox.desktop",
                "Firefox",
                "Browse the web",
                Kind::Application,
                &["internet"],
            ),
            item(
                "konsole.desktop",
                "Terminal",
                "A command line",
                Kind::Application,
                &["konsole", "shell"],
            ),
            item(
                "settings.desktop",
                "System Settings",
                "Configure Plasma",
                Kind::Application,
                &["preferences"],
            ),
            item(
                "monitor.desktop",
                "System Monitor",
                "Inspect processes",
                Kind::Application,
                &["cpu"],
            ),
            item("dir.home", "Home", "/home/test", Kind::Folder, &["user"]),
            item(
                "web.rust",
                "Rust Standard Library",
                "https://doc.rust-lang.org/std/",
                Kind::Web,
                &["docs"],
            ),
            item(
                "scene.about",
                "About Scene",
                "What this build can do",
                Kind::Scene,
                &["version"],
            ),
        ]
    }

    fn titles(query: &str, items: &[Item]) -> Vec<String> {
        search(query, items, &History::disabled())
            .into_iter()
            .map(|i| items[i].title.clone())
            .collect()
    }

    #[test]
    fn empty_query_returns_everything_grouped_by_kind() {
        let items = fixture();
        let hits = search("", &items, &History::disabled());
        assert_eq!(hits.len(), items.len());

        let priorities: Vec<u8> = hits.iter().map(|&i| items[i].kind.priority()).collect();
        assert!(
            priorities.windows(2).all(|w| w[0] <= w[1]),
            "groups out of order"
        );
    }

    #[test]
    fn a_tight_match_wins_the_top_slot() {
        let items = fixture();
        assert_eq!(
            titles("fire", &items).first().map(String::as_str),
            Some("Firefox")
        );
    }

    #[test]
    fn non_matching_query_returns_nothing() {
        assert!(search("zzzqqq", &fixture(), &History::disabled()).is_empty());
    }

    #[test]
    fn keywords_match_but_rank_below_titles() {
        let items = fixture();
        assert_eq!(
            titles("konsole", &items),
            vec!["Terminal"],
            "keyword-only match should be found"
        );

        // "settings" is in the title of one item and a keyword of none.
        assert_eq!(
            titles("settings", &items).first().map(String::as_str),
            Some("System Settings")
        );
    }

    #[test]
    fn a_description_matches_but_ranks_below_a_keyword() {
        let items = fixture();
        // "internet" is a keyword of Firefox; nothing has it in a title.
        assert_eq!(
            titles("internet", &items).first().map(String::as_str),
            Some("Firefox")
        );

        // "command" appears only in Terminal's description.
        assert!(titles("command", &items).contains(&"Terminal".to_string()));
    }

    #[test]
    fn the_package_group_sorts_ahead_of_every_other_group() {
        let priorities: Vec<u8> = [
            Kind::Package,
            Kind::Application,
            Kind::Folder,
            Kind::Web,
            Kind::Scene,
        ]
        .map(Kind::priority)
        .to_vec();
        assert!(priorities.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn a_query_answer_leads_the_results_it_was_generated_for() {
        // A provider's answer to "install firefox" must not be buried under
        // the installed Firefox, which matches the same words.
        let items = fixture();
        let answer = Item {
            id: "packages.install".to_string(),
            provider: "packages".into(),
            provider_title: "Packages".into(),
            provider_priority: 0,
            title: "Install “firefox”".to_string(),
            subtitle: "pkexec dnf install --assumeyes firefox".to_string(),
            kind: Kind::Package,
            icon: None,
            category: None,
            keywords: words(&["install firefox", "firefox", "package"]),
            action: Action::Message {
                text: "install".to_string(),
            },
            secondary_actions: Vec::new(),
        };

        // The same shape the launcher ranks: answers first, then the index.
        let answers = [answer];
        let visible: Vec<&Item> = answers.iter().chain(items.iter()).collect();
        let hits = search("install firefox", &visible, &History::disabled());

        assert_eq!(
            visible[hits[0]].title, "Install “firefox”",
            "the answer should lead"
        );

        // With no package query, the index ranks exactly as it did before.
        let plain: Vec<&Item> = items.iter().collect();
        assert_eq!(
            search("fire", &plain, &History::disabled())
                .into_iter()
                .map(|i| plain[i].title.clone())
                .collect::<Vec<_>>(),
            titles("fire", &items)
        );
    }

    #[test]
    fn ranking_is_stable_across_runs() {
        let items = fixture();
        assert_eq!(
            search("do", &items, &History::disabled()),
            search("do", &items, &History::disabled())
        );
    }

    #[test]
    fn search_ignores_case_and_surrounding_space() {
        let items = fixture();
        assert_eq!(titles("  FiReFox ", &items), titles("firefox", &items));
    }

    #[test]
    fn adjacent_characters_beat_gaps() {
        assert!(fuzzy("ab", "abc").unwrap() > fuzzy("ab", "axxxb").unwrap());
    }

    #[test]
    fn word_starts_beat_mid_word_matches() {
        assert!(fuzzy("m", "a monitor").unwrap() > fuzzy("m", "amonitor").unwrap());
    }

    /// A history with a stated clock and stated contents. It has no path, so
    /// no test can reach the user's own state file.
    fn history(now: u64, used: &[(&str, u32, u64)]) -> History {
        History {
            enabled: true,
            path: None,
            now,
            entries: used
                .iter()
                .map(|(id, count, last)| {
                    (
                        (*id).to_string(),
                        Use {
                            count: *count,
                            last: *last,
                        },
                    )
                })
                .collect(),
        }
    }

    const NOW: u64 = 1_700_000_000;
    const HOUR: u64 = 60 * 60;
    const DAY: u64 = 24 * HOUR;

    /// The fixture plus enough applications to pass the resting limit.
    fn crowded() -> Vec<Item> {
        let mut items = fixture();
        for index in 0..10 {
            items.push(Item {
                id: format!("app{index}.desktop"),
                provider: "applications".into(),
                provider_title: "Applications".into(),
                provider_priority: 1,
                title: format!("Zebra {index}"),
                subtitle: "A discovered application".into(),
                kind: Kind::Application,
                icon: None,
                category: None,
                keywords: words(&["zebra"]),
                action: Action::Message {
                    text: "zebra".to_string(),
                },
                secondary_actions: Vec::new(),
            });
        }
        items
    }

    #[test]
    fn the_resting_list_shows_only_the_top_applications() {
        let items = crowded();
        let applications = items
            .iter()
            .filter(|item| item.kind == Kind::Application)
            .count();
        let limit = Kind::Application
            .resting_limit()
            .expect("applications are trimmed at rest");
        assert!(applications > limit, "the fixture must exceed the limit");

        let resting = search("", &items, &History::disabled());
        let shown = resting
            .iter()
            .filter(|&&i| items[i].kind == Kind::Application)
            .count();
        assert_eq!(shown, limit);

        // Every other group is Scene's own catalogue, and shows in full.
        for kind in [Kind::Folder, Kind::Web, Kind::Scene] {
            let all = items.iter().filter(|item| item.kind == kind).count();
            let shown = resting.iter().filter(|&&i| items[i].kind == kind).count();
            assert_eq!(shown, all, "{kind:?} was trimmed");
        }
    }

    #[test]
    fn a_query_still_reaches_every_application() {
        // The trim is the resting state only. Searching must never hide a
        // result the user asked for by name.
        let items = crowded();
        let zebras = search("zebra", &items, &History::disabled());
        assert_eq!(zebras.len(), 10, "a query was trimmed");
        assert!(!search("Zebra 9", &items, &History::disabled()).is_empty());
    }

    #[test]
    fn use_decides_which_applications_rest_at_the_top() {
        let items = crowded();
        // The last one alphabetically, which nothing else would surface.
        let used = history(NOW, &[("app9.desktop", 5, NOW - HOUR / 2)]);
        let resting: Vec<String> = search("", &items, &used)
            .into_iter()
            .map(|i| items[i].title.clone())
            .collect();
        assert!(
            resting.contains(&"Zebra 9".to_string()),
            "a used application should rest in the visible five: {resting:?}"
        );
    }

    #[test]
    fn a_used_result_rises_within_its_group() {
        let items = fixture();
        // "System Settings" wins "system" on the baseline: it is the shorter
        // title and the match starts the word in both.
        assert_eq!(
            titles("system", &items).first().map(String::as_str),
            Some("System Settings")
        );

        let used = history(NOW, &[("monitor.desktop", 6, NOW - HOUR / 2)]);
        let ranked: Vec<String> = search("system", &items, &used)
            .into_iter()
            .map(|i| items[i].title.clone())
            .collect();
        assert_eq!(ranked.first().map(String::as_str), Some("System Monitor"));
    }

    #[test]
    fn history_never_reorders_the_groups() {
        let items = fixture();
        // Heavy use of a Scene command must not lift it above an application.
        let used = history(NOW, &[("scene.about", 99, NOW)]);
        let priorities: Vec<u8> = search("", &items, &used)
            .into_iter()
            .map(|i| items[i].kind.priority())
            .collect();
        assert!(
            priorities.windows(2).all(|w| w[0] <= w[1]),
            "history crossed a group boundary: {priorities:?}"
        );
    }

    #[test]
    fn history_cannot_lift_a_keyword_match_over_a_title_match() {
        // The bound that makes history an adjustment rather than a rewrite:
        // the largest possible bonus stays under the 40-point field penalty.
        assert!(frequency_bonus(u32::MAX) + MAX_RECENCY_BONUS < 40);

        let items = fixture();
        let used = history(NOW, &[("konsole.desktop", u32::MAX, NOW)]);
        // "settings" is System Settings' title and nothing else's keyword, so
        // no amount of use may displace it.
        assert_eq!(
            search("settings", &items, &used)
                .into_iter()
                .map(|i| items[i].title.clone())
                .next(),
            Some("System Settings".to_string())
        );
    }

    #[test]
    fn an_older_use_counts_for_less_than_a_recent_one() {
        let recent = history(NOW, &[("dir.home", 1, NOW - HOUR / 2)]);
        let older = history(NOW, &[("dir.home", 1, NOW - 10 * DAY)]);
        let ancient = history(NOW, &[("dir.home", 1, NOW - 400 * DAY)]);
        assert!(recent.bonus("dir.home") > older.bonus("dir.home"));
        assert!(older.bonus("dir.home") > ancient.bonus("dir.home"));
        assert_eq!(recent.bonus("never.chosen"), 0);
    }

    #[test]
    fn a_disabled_history_scores_nothing_and_records_nothing() {
        let mut disabled = History::disabled();
        disabled.record("dir.home");
        assert_eq!(disabled.bonus("dir.home"), 0);

        let items = fixture();
        assert_eq!(
            search("system", &items, &disabled),
            search("system", &items, &History::disabled())
        );
    }

    #[test]
    fn ranking_with_a_history_is_still_stable_across_runs() {
        let items = fixture();
        let used = history(
            NOW,
            &[("monitor.desktop", 3, NOW - DAY), ("dir.home", 9, NOW)],
        );
        assert_eq!(
            search("o", &items, &used),
            search("o", &items, &used),
            "the same query, items and history must give the same order"
        );
    }

    #[test]
    fn a_history_survives_being_written_and_read_back() {
        let used = history(
            NOW,
            &[("firefox.desktop", 4, NOW - DAY), ("dir.home", 1, NOW)],
        );
        let read = History::parse(&used.render(), NOW);
        assert_eq!(read.entries, used.entries);
        assert_eq!(read.bonus("firefox.desktop"), used.bonus("firefox.desktop"));
    }

    #[test]
    fn an_unreadable_history_is_no_history_rather_than_an_error() {
        for text in [
            "",
            "not a scene history
1 2 firefox.desktop
",
            "scene-history 2
1 2 firefox.desktop
",
        ] {
            assert!(History::parse(text, NOW).entries.is_empty(), "{text:?}");
        }

        // A damaged line is skipped; the sound lines around it are kept.
        let salvaged = History::parse(
            "scene-history 1
rubbish
2 nonsense id
3 1700 dir.home
",
            NOW,
        );
        assert_eq!(salvaged.entries.len(), 1);
        assert!(salvaged.entries.contains_key("dir.home"));
    }

    #[test]
    fn the_history_file_is_bounded() {
        let mut full = History::parse(
            "scene-history 1
",
            NOW,
        );
        for index in 0..HISTORY_LIMIT + 40 {
            full.entries.insert(
                format!("item.{index}"),
                Use {
                    count: 1,
                    last: NOW - index as u64,
                },
            );
        }
        full.forget_oldest();
        assert_eq!(full.entries.len(), HISTORY_LIMIT);
        // The oldest entries are the ones dropped.
        assert!(full.entries.contains_key("item.0"));
        assert!(
            !full
                .entries
                .contains_key(&format!("item.{}", HISTORY_LIMIT + 39))
        );
    }

    #[test]
    fn history_is_switched_off_by_the_environment() {
        for value in ["off", "OFF", " off ", "0", "no", "false"] {
            assert!(!enabled_by(Some(value)), "{value:?}");
        }
        for value in ["on", "1", ""] {
            assert!(enabled_by(Some(value)), "{value:?}");
        }
        assert!(enabled_by(None), "history is on unless it is switched off");
    }

    #[test]
    fn a_recorded_use_reaches_the_file_and_comes_back() {
        // An explicit path, so this never touches the user's state directory.
        let path =
            std::env::temp_dir().join(format!("scene-history-test-{}-{}", std::process::id(), NOW));
        let mut writing = History {
            enabled: true,
            path: Some(path.clone()),
            now: NOW,
            entries: BTreeMap::new(),
        };
        writing.record("dir.home");
        writing.record("dir.home");
        writing.record("firefox.desktop");

        let text = std::fs::read_to_string(&path).expect("the history was written");
        std::fs::remove_file(&path).expect("remove the test history");

        let read = History::parse(&text, writing.now);
        assert_eq!(read.entries.len(), 2);
        assert_eq!(read.entries["dir.home"].count, 2);
        assert_eq!(read.entries["firefox.desktop"].count, 1);
        assert!(read.bonus("dir.home") > read.bonus("firefox.desktop"));
    }
}
