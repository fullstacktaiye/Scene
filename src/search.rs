//! Query matching and ranking over an in-memory result set.
//!
//! Milestone 1 has one provider: a static catalogue. Ranking is deterministic
//! so the same query always produces the same order.

use crate::actions::Action;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Application,
    Folder,
    Web,
    Scene,
}

impl Kind {
    /// Groups appear in this order, best first.
    fn priority(self) -> u8 {
        match self {
            Kind::Application => 0,
            Kind::Folder => 1,
            Kind::Web => 2,
            Kind::Scene => 3,
        }
    }

    pub fn heading(self) -> &'static str {
        match self {
            Kind::Application => "APPLICATIONS",
            Kind::Folder => "FOLDERS",
            Kind::Web => "WEB",
            Kind::Scene => "SCENE",
        }
    }

    pub fn tag(self) -> &'static str {
        match self {
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
            Kind::Application => "application",
            Kind::Folder => "folder",
            Kind::Web => "web",
            Kind::Scene => "scene",
        }
    }

    pub fn fallback_icon(self) -> &'static str {
        match self {
            Kind::Application => "application-x-executable-symbolic",
            Kind::Folder => "folder-symbolic",
            Kind::Web => "web-browser-symbolic",
            Kind::Scene => "starred-symbolic",
        }
    }
}

pub struct Item {
    pub id: &'static str,
    pub title: String,
    pub subtitle: String,
    pub kind: Kind,
    /// Icon theme name; falls back to `kind.fallback_icon()`.
    pub icon: &'static str,
    pub keywords: &'static [&'static str],
    pub action: Action,
}

/// Returns the indices of `items` that match `query`, grouped by kind and
/// ranked within each group. An empty query matches everything.
pub fn search(query: &str, items: &[Item]) -> Vec<usize> {
    let needle = query.trim().to_lowercase();

    let mut hits: Vec<(u8, i32, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let score = if needle.is_empty() {
                0
            } else {
                score(&needle, item)?
            };
            Some((item.kind.priority(), -score, i))
        })
        .collect();

    // Group, then best score, then the order the provider listed them in.
    hits.sort_unstable();
    hits.into_iter().map(|(_, _, i)| i).collect()
}

/// Best score across an item's searchable text, or `None` if nothing matches.
fn score(needle: &str, item: &Item) -> Option<i32> {
    let title = fuzzy(needle, &item.title.to_lowercase());
    let keywords = item
        .keywords
        .iter()
        .filter_map(|k| fuzzy(needle, &k.to_lowercase()))
        .max()
        // A keyword hit is real, but it should never outrank a title hit.
        .map(|s| s - 40);
    title.max(keywords)
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

/// The Milestone 1 provider: a fixed set of results, resolved at startup.
pub fn catalogue() -> Vec<Item> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());

    let app = |id, title: &str, subtitle: &str, program: &str, icon, keywords| Item {
        id,
        title: title.to_string(),
        subtitle: subtitle.to_string(),
        kind: Kind::Application,
        icon,
        keywords,
        action: Action::Run {
            program: program.to_string(),
            args: Vec::new(),
        },
    };

    let folder = |id, title: &str, path: String, keywords| Item {
        id,
        title: title.to_string(),
        subtitle: path.clone(),
        kind: Kind::Folder,
        icon: "folder",
        keywords,
        action: Action::Open { target: path },
    };

    let link = |id, title: &str, url: &str, keywords| Item {
        id,
        title: title.to_string(),
        subtitle: url.to_string(),
        kind: Kind::Web,
        icon: "web-browser",
        keywords,
        action: Action::Open {
            target: url.to_string(),
        },
    };

    vec![
        app("app.files", "Files", "Browse your files with Dolphin", "dolphin",
            "system-file-manager", &["dolphin", "file manager", "browse"]),
        app("app.terminal", "Terminal", "Open a Konsole window", "konsole",
            "utilities-terminal", &["konsole", "shell", "console", "cli"]),
        app("app.firefox", "Firefox", "Browse the web", "firefox",
            "firefox", &["browser", "web", "internet"]),
        app("app.editor", "Text Editor", "Edit text with Kate", "kate",
            "accessories-text-editor", &["kate", "text", "code", "notes"]),
        app("app.settings", "System Settings", "Configure KDE Plasma", "systemsettings",
            "preferences-system", &["kde", "plasma", "preferences", "configure"]),
        app("app.monitor", "System Monitor", "Inspect processes and load", "plasma-systemmonitor",
            "utilities-system-monitor", &["processes", "cpu", "memory", "task manager"]),
        app("app.calculator", "Calculator", "Do some arithmetic", "kcalc",
            "accessories-calculator", &["kcalc", "maths", "sums"]),

        folder("dir.home", "Home", home.clone(), &["~", "user"]),
        folder("dir.downloads", "Downloads", format!("{home}/Downloads"), &["saved", "files"]),
        folder("dir.documents", "Documents", format!("{home}/Documents"), &["docs", "papers"]),

        link("web.gtk", "GTK 4 Documentation", "https://docs.gtk.org/gtk4/",
             &["gtk", "docs", "reference", "toolkit"]),
        link("web.rust", "Rust Standard Library", "https://doc.rust-lang.org/std/",
             &["rust", "docs", "std", "reference"]),

        Item {
            id: "scene.about",
            title: "About Scene".into(),
            subtitle: "What this build can do".into(),
            kind: Kind::Scene,
            icon: "help-about",
            keywords: &["version", "help", "info"],
            action: Action::Message {
                text: concat!(
                    "Scene ", env!("CARGO_PKG_VERSION"),
                    " — Milestone 1. The launcher shell, searching a static result set."
                ).into(),
            },
        },
        Item {
            id: "scene.keys",
            title: "Keyboard Shortcuts".into(),
            subtitle: "How to drive Scene without a mouse".into(),
            kind: Kind::Scene,
            icon: "input-keyboard",
            keywords: &["keys", "bindings", "navigation", "help"],
            action: Action::Message {
                text: "Up and Down to move, Enter to open, Escape to clear then close, Ctrl+Q to quit.".into(),
            },
        },
        Item {
            id: "scene.quit",
            title: "Quit Scene".into(),
            subtitle: "Stop the launcher entirely".into(),
            kind: Kind::Scene,
            icon: "application-exit",
            keywords: &["exit", "close", "stop"],
            action: Action::Quit,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(query: &str, items: &[Item]) -> Vec<String> {
        search(query, items)
            .into_iter()
            .map(|i| items[i].title.clone())
            .collect()
    }

    #[test]
    fn empty_query_returns_everything_grouped_by_kind() {
        let items = catalogue();
        let hits = search("", &items);
        assert_eq!(hits.len(), items.len());

        let priorities: Vec<u8> = hits.iter().map(|&i| items[i].kind.priority()).collect();
        assert!(
            priorities.windows(2).all(|w| w[0] <= w[1]),
            "groups out of order"
        );
    }

    #[test]
    fn a_tight_match_wins_the_top_slot() {
        let items = catalogue();
        assert_eq!(
            titles("fire", &items).first().map(String::as_str),
            Some("Firefox")
        );
    }

    #[test]
    fn non_matching_query_returns_nothing() {
        assert!(search("zzzqqq", &catalogue()).is_empty());
    }

    #[test]
    fn keywords_match_but_rank_below_titles() {
        let items = catalogue();
        let hits = titles("konsole", &items);
        assert_eq!(hits, vec!["Terminal"], "keyword-only match should be found");

        // "settings" is in the title of one item and a keyword of none.
        assert_eq!(
            titles("settings", &items).first().map(String::as_str),
            Some("System Settings")
        );
    }

    #[test]
    fn ranking_is_stable_across_runs() {
        let items = catalogue();
        assert_eq!(search("do", &items), search("do", &items));
    }

    #[test]
    fn search_ignores_case_and_surrounding_space() {
        let items = catalogue();
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
}
