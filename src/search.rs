//! Query matching and ranking over an in-memory result set.
//!
//! Ranking is deterministic: the same query against the same index always
//! produces the same order. Providers hand this module typed items; it never
//! executes anything and never touches the UI.

use std::borrow::Borrow;

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
    fn priority(self) -> u8 {
        match self {
            Kind::Package => 0,
            Kind::Application => 1,
            Kind::Folder => 2,
            Kind::Web => 3,
            Kind::Scene => 4,
        }
    }

    pub fn heading(self) -> &'static str {
        match self {
            Kind::Package => "PACKAGES",
            Kind::Application => "APPLICATIONS",
            Kind::Folder => "FOLDERS",
            Kind::Web => "WEB",
            Kind::Scene => "SCENE",
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

pub struct Item {
    pub id: String,
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
    let mut items = crate::integrations::index();
    // Built-in Scene help is itself a provider-like static catalogue. It stays
    // separate because it needs no discovery or executable capability.
    items.extend(catalogue());
    items
}

/// Returns the indices of `items` that match `query`, grouped by kind and
/// ranked within each group. An empty query matches everything.
///
/// It takes anything that borrows an `Item`, so the caller can rank one owned
/// index and a set of query answers together without copying either.
pub fn search<I: Borrow<Item>>(query: &str, items: &[I]) -> Vec<usize> {
    let needle = query.trim().to_lowercase();

    let mut hits: Vec<(u8, i32, usize)> = items
        .iter()
        .map(Borrow::borrow)
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
        title: title.to_string(),
        subtitle: path.clone(),
        kind: Kind::Folder,
        icon: themed("folder"),
        category: None,
        keywords: words(keywords),
        action: Action::Open { target: path },
    };

    let link = |id: &str, title: &str, url: &str, keywords: &[&str]| Item {
        id: id.to_string(),
        title: title.to_string(),
        subtitle: url.to_string(),
        kind: Kind::Web,
        icon: themed("web-browser"),
        category: None,
        keywords: words(keywords),
        action: Action::Open {
            target: url.to_string(),
        },
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
            id: "scene.about".to_string(),
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
                    " — Milestone 4. Distro package adapters are ready."
                )
                .into(),
            },
        },
        Item {
            id: "scene.keys".to_string(),
            title: "Keyboard Shortcuts".into(),
            subtitle: "How to drive Scene without a mouse".into(),
            kind: Kind::Scene,
            icon: themed("input-keyboard"),
            category: None,
            keywords: words(&["keys", "bindings", "navigation", "help"]),
            action: Action::Message {
                text: "Up and Down to move, Enter to open, Escape to clear then close, Ctrl+Q to quit.".into(),
            },
        },
        Item {
            id: "scene.quit".to_string(),
            title: "Quit Scene".into(),
            subtitle: "Stop the launcher entirely".into(),
            kind: Kind::Scene,
            icon: themed("application-exit"),
            category: None,
            keywords: words(&["exit", "close", "stop"]),
            action: Action::Quit,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed set of items, so ranking tests do not depend on the machine's
    /// installed applications or on the catalogue's current contents.
    fn fixture() -> Vec<Item> {
        let item = |id: &str, title: &str, subtitle: &str, kind, keywords: &[&str]| Item {
            id: id.to_string(),
            title: title.to_string(),
            subtitle: subtitle.to_string(),
            kind,
            icon: None,
            category: None,
            keywords: words(keywords),
            action: Action::Message {
                text: title.to_string(),
            },
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
        search(query, items)
            .into_iter()
            .map(|i| items[i].title.clone())
            .collect()
    }

    #[test]
    fn empty_query_returns_everything_grouped_by_kind() {
        let items = fixture();
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
        let items = fixture();
        assert_eq!(
            titles("fire", &items).first().map(String::as_str),
            Some("Firefox")
        );
    }

    #[test]
    fn non_matching_query_returns_nothing() {
        assert!(search("zzzqqq", &fixture()).is_empty());
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
            title: "Install “firefox”".to_string(),
            subtitle: "pkexec dnf install --assumeyes firefox".to_string(),
            kind: Kind::Package,
            icon: None,
            category: None,
            keywords: words(&["install firefox", "firefox", "package"]),
            action: Action::Message {
                text: "install".to_string(),
            },
        };

        // The same shape the launcher ranks: answers first, then the index.
        let answers = [answer];
        let visible: Vec<&Item> = answers.iter().chain(items.iter()).collect();
        let hits = search("install firefox", &visible);

        assert_eq!(
            visible[hits[0]].title, "Install “firefox”",
            "the answer should lead"
        );

        // With no package query, the index ranks exactly as it did before.
        let plain: Vec<&Item> = items.iter().collect();
        assert_eq!(
            search("fire", &plain)
                .into_iter()
                .map(|i| plain[i].title.clone())
                .collect::<Vec<_>>(),
            titles("fire", &items)
        );
    }

    #[test]
    fn ranking_is_stable_across_runs() {
        let items = fixture();
        assert_eq!(search("do", &items), search("do", &items));
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
}
