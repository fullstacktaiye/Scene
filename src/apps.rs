//! Installed application discovery.
//!
//! Applications come from the desktop's own application model. GIO already
//! knows which `.desktop` entries exist, which are hidden, and which belong in
//! this session, so Scene does not parse them itself. The one thing GIO's Rust
//! bindings do not expose is `GDesktopAppInfo`, so the remaining fields Scene
//! wants — generic name, keywords, categories — are read from the entry with
//! `KeyFile`.

use gtk::gio::prelude::*;
use gtk::{gio, glib};

use crate::actions::Action;
use crate::search::{Item, Kind};

/// Every installed application the desktop says belongs in this session.
///
/// Measured on the development machine: 292 desktop entries, 81 of them
/// shown, in 12-19 ms warm and 27 ms cold. That runs at startup before any
/// window exists, and again only when the installed set actually changes, so
/// it never blocks a visible UI and does not need a thread.
pub fn installed() -> Vec<Item> {
    let mut items: Vec<Item> = gio::AppInfo::all()
        .into_iter()
        // Honours NoDisplay, Hidden, OnlyShowIn and NotShowIn for us.
        .filter(|app| app.should_show())
        .map(item)
        .collect();

    // `AppInfo::all` promises no particular order, and ranking has to be
    // reproducible, so impose one.
    items.sort_by(|a, b| {
        a.title
            .to_lowercase()
            .cmp(&b.title.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    items
}

fn item(app: gio::AppInfo) -> Item {
    let id = app.id().map(|id| id.to_string()).unwrap_or_default();
    let entry = Entry::read(&id);
    let executable = executable_name(&app);

    let title = app.display_name().to_string();
    let subtitle = app
        .description()
        .map(|text| text.to_string())
        .filter(|text| !text.is_empty())
        .or_else(|| entry.generic_name.clone())
        .unwrap_or_else(|| executable.clone());

    // Everything a user might plausibly type that is not the title.
    let mut keywords = entry.keywords;
    keywords.extend(entry.generic_name);
    keywords.extend(entry.categories.iter().cloned());
    keywords.push(executable);

    Item {
        icon: app.icon(),
        category: category(&entry.categories),
        action: Action::Launch { app },
        id,
        title,
        subtitle,
        kind: Kind::Application,
        keywords,
    }
}

/// The freedesktop main categories. An entry lists several, mixing main and
/// additional ones; the first main category is the one worth showing.
const MAIN_CATEGORIES: &[&str] = &[
    "AudioVideo",
    "Audio",
    "Video",
    "Development",
    "Education",
    "Game",
    "Graphics",
    "Network",
    "Office",
    "Science",
    "Settings",
    "System",
    "Utility",
];

fn category(categories: &[String]) -> Option<String> {
    categories
        .iter()
        .find(|c| MAIN_CATEGORIES.contains(&c.as_str()))
        .map(|c| match c.as_str() {
            "AudioVideo" => "Audio & Video".to_string(),
            other => other.to_string(),
        })
}

fn executable_name(app: &gio::AppInfo) -> String {
    app.executable()
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The parts of a `.desktop` entry that GIO's Rust bindings do not expose.
#[derive(Default)]
struct Entry {
    generic_name: Option<String>,
    keywords: Vec<String>,
    categories: Vec<String>,
}

impl Entry {
    /// An entry that cannot be found or parsed simply yields no extra detail;
    /// the application is still listed and still launches.
    fn read(id: &str) -> Self {
        if id.is_empty() {
            return Self::default();
        }

        let file = glib::KeyFile::new();
        // Searches the XDG data directories in the order the desktop uses.
        if file
            .load_from_data_dirs(format!("applications/{id}"), glib::KeyFileFlags::NONE)
            .is_err()
        {
            return Self::default();
        }

        const GROUP: &str = "Desktop Entry";
        Self {
            generic_name: file
                .locale_string(GROUP, "GenericName", None)
                .ok()
                .map(|name| name.to_string())
                .filter(|name| !name.is_empty()),
            keywords: list(&file, GROUP, "Keywords"),
            categories: list(&file, GROUP, "Categories"),
        }
    }
}

fn list(file: &glib::KeyFile, group: &str, key: &str) -> Vec<String> {
    file.string_list(group, key)
        .map(|values| values.iter().map(|value| value.to_string()).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_main_category_becomes_the_label() {
        let categories = |list: &[&str]| list.iter().map(|c| c.to_string()).collect::<Vec<_>>();

        // "WebBrowser" is an additional category, "Network" is a main one.
        assert_eq!(
            category(&categories(&["Network", "WebBrowser"])),
            Some("Network".to_string())
        );
        assert_eq!(
            category(&categories(&["WebBrowser", "Network"])),
            Some("Network".to_string())
        );
        assert_eq!(category(&categories(&["WebBrowser"])), None);
        assert_eq!(category(&[]), None);
    }

    #[test]
    fn audio_video_reads_as_words() {
        assert_eq!(
            category(&["AudioVideo".to_string()]),
            Some("Audio & Video".to_string())
        );
    }

    #[test]
    fn a_missing_desktop_entry_yields_no_detail() {
        let entry = Entry::read("dev.scene.definitely-not-installed.desktop");
        assert!(entry.generic_name.is_none());
        assert!(entry.keywords.is_empty());
        assert!(entry.categories.is_empty());
    }
}
