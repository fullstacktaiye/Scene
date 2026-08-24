//! Built-in integration contracts and registry.
//!
//! Providers may return an error, but the registry converts it to one local
//! unavailable result. A faulty provider can therefore never remove another
//! provider's results or break the launcher surface.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{Local, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use glib::variant::ToVariant;
use gtk::gio::prelude::FileExt;
use gtk::{gio, glib};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};

use crate::actions::{
    Action, Bus, Confirmation, DbusAction, DbusArguments, ProcessAction, SignalAction,
};
use crate::packages::{self, Capability, Detected, Term, Unsupported};
use crate::platform::DesktopSupport;
use crate::search::{self, Item, Kind};
use crate::system::{self, CommandSpec};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Metadata {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub default_priority: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderPreference {
    pub enabled: bool,
    pub priority: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectoryConfig {
    pub path: PathBuf,
}

/// Configuration is explicit, typed, versioned, and persisted under the XDG
/// configuration directory. Environment overrides remain useful for tests and
/// one-off sessions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub directory: DirectoryConfig,
    pub providers: BTreeMap<String, ProviderPreference>,
    pub history_enabled: bool,
    pub command_history_enabled: bool,
    pub file_content_enabled: bool,
}

impl Config {
    pub fn load() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"));
        let file = glib::KeyFile::new();
        if let Some(path) = config_path() {
            let _ = file.load_from_file(path, glib::KeyFileFlags::NONE);
        }
        let configured_directory = file
            .string("general", "directory")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| home.clone());
        let directory = std::env::var_os("SCENE_DIRECTORY")
            .map(PathBuf::from)
            .unwrap_or(configured_directory);
        let history_enabled = std::env::var("SCENE_HISTORY")
            .ok()
            .map(|value| {
                !matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "off" | "0" | "no" | "false"
                )
            })
            .unwrap_or_else(|| file.boolean("general", "history-enabled").unwrap_or(true));

        let mut providers = BTreeMap::new();
        for metadata in provider_metadata() {
            let group = format!("provider {}", metadata.id);
            providers.insert(
                metadata.id.to_string(),
                ProviderPreference {
                    enabled: file.boolean(&group, "enabled").unwrap_or(true),
                    priority: file
                        .integer(&group, "priority")
                        .ok()
                        .and_then(|value| u16::try_from(value).ok())
                        .unwrap_or(metadata.default_priority),
                },
            );
        }
        Self {
            directory: DirectoryConfig { path: directory },
            providers,
            history_enabled,
            command_history_enabled: file
                .boolean("general", "command-history-enabled")
                .unwrap_or(false),
            file_content_enabled: file
                .boolean("general", "file-content-enabled")
                .unwrap_or(false),
        }
    }

    pub fn provider_enabled(&self, id: &str) -> bool {
        self.providers
            .get(id)
            .is_none_or(|provider| provider.enabled)
    }

    pub fn provider_priority(&self, id: &str) -> u16 {
        self.providers
            .get(id)
            .map(|provider| provider.priority)
            .unwrap_or(u16::MAX)
    }

    pub fn set_provider_enabled(&mut self, id: &str, enabled: bool) {
        if let Some(provider) = self.providers.get_mut(id) {
            provider.enabled = enabled;
        }
    }

    pub fn move_provider(&mut self, id: &str, delta: i32) {
        let mut ordered = self.ordered_provider_ids();
        let Some(position) = ordered.iter().position(|candidate| candidate == id) else {
            return;
        };
        let next =
            (position as i32 + delta).clamp(0, ordered.len().saturating_sub(1) as i32) as usize;
        if next == position {
            return;
        }
        ordered.swap(position, next);
        for (priority, provider) in ordered.into_iter().enumerate() {
            if let Some(preference) = self.providers.get_mut(&provider) {
                preference.priority = priority as u16;
            }
        }
    }

    pub fn ordered_provider_ids(&self) -> Vec<String> {
        let mut providers = self.providers.iter().collect::<Vec<_>>();
        providers.sort_by_key(|(id, preference)| (preference.priority, (*id).clone()));
        providers.into_iter().map(|(id, _)| id.clone()).collect()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = config_path() else {
            return Ok(());
        };
        atomic_write(&path, self.key_file().to_data().as_bytes())
    }

    fn key_file(&self) -> glib::KeyFile {
        let file = glib::KeyFile::new();
        file.set_integer("format", "version", 1);
        file.set_string(
            "general",
            "directory",
            &self.directory.path.to_string_lossy(),
        );
        file.set_boolean("general", "history-enabled", self.history_enabled);
        file.set_boolean(
            "general",
            "command-history-enabled",
            self.command_history_enabled,
        );
        file.set_boolean("general", "file-content-enabled", self.file_content_enabled);
        for (id, provider) in &self.providers {
            let group = format!("provider {id}");
            file.set_boolean(&group, "enabled", provider.enabled);
            file.set_integer(&group, "priority", i32::from(provider.priority));
        }
        file
    }
}

fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("scene").join("config.ini"))
}

fn atomic_write(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&temporary, contents)?;
    std::fs::rename(temporary, path)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntegrationError {
    pub message: String,
}

/// The search contract: each provider returns stable, renderable items for the
/// central deterministic matcher. It cannot return widgets or commands.
pub trait Integration {
    fn metadata(&self) -> Metadata;

    /// Items that exist regardless of what the user typed. Collected once and
    /// re-collected only when the installed set changes.
    fn search(&self, config: &Config) -> Result<Vec<Item>, IntegrationError>;

    /// Items that answer one specific query — a package name, for instance,
    /// which no static index can hold.
    ///
    /// This runs on the GTK thread for every keystroke, so it may *build* a
    /// command but must never run one. Execution stays where it belongs: in
    /// `actions`, off the UI thread, bounded and cancellable.
    fn answer(&self, _query: &str, _config: &Config) -> Result<Vec<Item>, IntegrationError> {
        Ok(Vec::new())
    }
}

const PROVIDERS: [&dyn Integration; 26] = [
    &Calculator,
    &Currency,
    &DateAndTime,
    &Colors,
    &Characters,
    &Commands,
    &Windows,
    &Files,
    &Activities,
    &SystemSettings,
    &PowerAndSession,
    &Processes,
    &GlobalShortcuts,
    &RecentDocuments,
    &KdePlaces,
    &Bookmarks,
    &DeclinedCapabilities,
    &BuiltinPlaces,
    &Documentation,
    &SceneCommands,
    &Applications,
    &Terminal,
    &SystemInformation,
    &ConfiguredDirectory,
    &WebShortcuts,
    &Packages,
];

pub fn provider_metadata() -> Vec<Metadata> {
    PROVIDERS.into_iter().map(Integration::metadata).collect()
}

/// Discover every built-in provider. Errors remain visible and local.
pub fn index() -> Vec<Item> {
    let config = Config::load();
    collect(&config, |provider| provider.search(&config))
}

/// The providers' answers to one query, ranked alongside the static index.
pub fn answers(query: &str) -> Vec<Item> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let config = Config::load();
    collect(&config, |provider| provider.answer(query, &config))
}

/// Providers backed by desktop services answer asynchronously. Each callback
/// is one provider's complete answer, so the UI can reject a stale generation
/// without waiting for unrelated services.
pub fn answers_async(query: &str, callback: impl Fn(Vec<Item>) + Clone + 'static) {
    let query = query.trim().to_string();
    if query.is_empty() {
        return;
    }
    let config = Config::load();
    if config.provider_enabled("currency")
        && let Some(request) = CurrencyRequest::parse(&query)
    {
        let callback = callback.clone();
        let priority = config.provider_priority("currency");
        glib::MainContext::default().spawn_local(async move {
            let result = gio::spawn_blocking(move || currency_answer(request)).await;
            let mut item = match result {
                Ok(Ok(answer)) => copy_answer(
                    "currency.answer",
                    &answer.value,
                    &answer.subtitle,
                    &answer.query,
                ),
                Ok(Err(message)) => answer_item(
                    "currency.error",
                    "Currency conversion unavailable",
                    &message,
                    Kind::Scene,
                    "dialog-warning-symbolic",
                    "currency",
                    Action::Message {
                        text: message.clone(),
                    },
                ),
                Err(_) => answer_item(
                    "currency.error",
                    "Currency conversion unavailable",
                    "The currency worker stopped unexpectedly.",
                    Kind::Scene,
                    "dialog-warning-symbolic",
                    "currency",
                    Action::Message {
                        text: "The currency worker stopped unexpectedly.".into(),
                    },
                ),
            };
            item.provider = "currency".into();
            item.provider_title = "Currency".into();
            item.provider_priority = priority;
            callback(vec![item]);
        });
    }
    if config.provider_enabled("packages")
        && let Some(term_text) = package_status_term(&query)
    {
        let callback = callback.clone();
        let priority = config.provider_priority("packages");
        let user_query = query.clone();
        glib::MainContext::default().spawn_local(async move {
            let result = gio::spawn_blocking(move || package_snapshot(&term_text)).await;
            let mut item = match result {
                Ok(Ok(snapshot)) => merged_package_item(snapshot, &user_query),
                Ok(Err(message)) => answer_item(
                    "packages.merged-error",
                    "Package status unavailable",
                    &message,
                    Kind::Package,
                    "dialog-warning-symbolic",
                    &user_query,
                    Action::Message {
                        text: message.clone(),
                    },
                ),
                Err(_) => answer_item(
                    "packages.merged-error",
                    "Package status unavailable",
                    "The package worker stopped unexpectedly.",
                    Kind::Package,
                    "dialog-warning-symbolic",
                    &user_query,
                    Action::Message {
                        text: "The package worker stopped unexpectedly.".into(),
                    },
                ),
            };
            item.provider = "packages".into();
            item.provider_title = "Packages".into();
            item.provider_priority = priority;
            callback(vec![item]);
        });
    }
    for runner in [
        RunnerProvider::windows(),
        RunnerProvider::files(),
        RunnerProvider::activities(),
    ] {
        if !config.provider_enabled(runner.metadata.id) {
            continue;
        }
        let callback = callback.clone();
        let original_query = query.clone();
        let service_query = if runner.metadata.id == "files" {
            let term = query.strip_prefix("file ").unwrap_or(&query);
            if config.file_content_enabled {
                term.to_string()
            } else {
                format!("filename:{term}")
            }
        } else {
            query.clone()
        };
        let priority = config.provider_priority(runner.metadata.id);
        glib::MainContext::default().spawn_local(async move {
            let answer = runner
                .query(&service_query, &original_query, priority)
                .await;
            callback(answer);
        });
    }
}

/// One provider's error becomes one local result, never a missing group.
fn collect(
    config: &Config,
    mut ask: impl FnMut(&dyn Integration) -> Result<Vec<Item>, IntegrationError>,
) -> Vec<Item> {
    PROVIDERS
        .into_iter()
        .filter(|provider| config.provider_enabled(provider.metadata().id))
        .flat_map(|provider| {
            let metadata = provider.metadata();
            match ask(provider) {
                Ok(mut items) => {
                    for item in &mut items {
                        item.provider = metadata.id.into();
                        item.provider_title = metadata.title.into();
                        item.provider_priority = config.provider_priority(metadata.id);
                    }
                    items
                }
                Err(error) => vec![unavailable_item(metadata, error)],
            }
        })
        .collect()
}

fn unavailable_item(metadata: Metadata, error: IntegrationError) -> Item {
    Item {
        id: format!("provider.{}.unavailable", metadata.id),
        provider: metadata.id.into(),
        provider_title: metadata.title.into(),
        provider_priority: metadata.default_priority,
        title: format!("{} unavailable", metadata.title),
        subtitle: error.message.clone(),
        kind: Kind::Scene,
        icon: search::themed("dialog-warning-symbolic"),
        category: Some(metadata.title.to_string()),
        keywords: vec![
            metadata.id.to_string(),
            "unavailable".into(),
            "error".into(),
        ],
        action: Action::Message {
            text: error.message,
        },
        secondary_actions: Vec::new(),
    }
}

struct Windows;
struct Files;
struct Activities;

macro_rules! async_provider {
    ($type:ty, $id:literal, $title:literal, $description:literal, $priority:literal) => {
        impl Integration for $type {
            fn metadata(&self) -> Metadata {
                Metadata {
                    id: $id,
                    title: $title,
                    description: $description,
                    default_priority: $priority,
                }
            }

            fn search(&self, _: &Config) -> Result<Vec<Item>, IntegrationError> {
                Ok(Vec::new())
            }
        }
    };
}

async_provider!(
    Windows,
    "windows",
    "Windows",
    "Switch to open Plasma windows",
    11
);
async_provider!(
    Files,
    "files",
    "Files",
    "Search the existing Baloo index",
    20
);
async_provider!(
    Activities,
    "activities",
    "Activities",
    "Switch KDE Plasma Activities",
    12
);

struct SystemSettings;

impl Integration for SystemSettings {
    fn metadata(&self) -> Metadata {
        Metadata {
            id: "system-settings",
            title: "System Settings",
            description: "Open a named KDE settings module",
            default_priority: 62,
        }
    }

    fn search(&self, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        if !system::executable_on_path("systemsettings") {
            return Err(IntegrationError {
                message: "KDE System Settings is not installed or not on PATH.".into(),
            });
        }
        let Ok(entries) = std::fs::read_dir("/usr/share/applications") else {
            return Ok(Vec::new());
        };
        let mut items = Vec::new();
        for entry in entries.flatten() {
            let filename = entry.file_name();
            let filename = filename.to_string_lossy();
            if !filename.starts_with("kcm_") || !filename.ends_with(".desktop") {
                continue;
            }
            let file = glib::KeyFile::new();
            if file
                .load_from_file(entry.path(), glib::KeyFileFlags::NONE)
                .is_err()
            {
                continue;
            }
            let Ok(title) = file.locale_string("Desktop Entry", "Name", None) else {
                continue;
            };
            let id = filename.trim_end_matches(".desktop").to_string();
            let icon = file
                .string("Desktop Entry", "Icon")
                .map(|value| value.to_string())
                .unwrap_or_else(|_| "preferences-system-symbolic".into());
            items.push(Item {
                id: format!("system-settings.{id}"),
                provider: String::new(),
                provider_title: String::new(),
                provider_priority: 0,
                title: title.to_string(),
                subtitle: format!("Open {id} in KDE System Settings"),
                kind: Kind::Scene,
                icon: search::themed(&icon),
                category: Some("Settings".into()),
                keywords: vec![id.clone(), "settings".into(), "preferences".into()],
                action: Action::Process {
                    action: ProcessAction::detached(
                        format!("system-settings.{id}"),
                        title.to_string(),
                        CommandSpec::read_only("systemsettings", [id]),
                    ),
                },
                secondary_actions: Vec::new(),
            });
        }
        items.sort_by_key(|item| item.title.to_lowercase());
        Ok(items)
    }
}

struct PowerAndSession;

impl Integration for PowerAndSession {
    fn metadata(&self) -> Metadata {
        Metadata {
            id: "power-session",
            title: "Power and session",
            description: "KDE and logind session actions",
            default_priority: 63,
        }
    }

    fn search(&self, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        if !DesktopSupport::detect().is_kde() {
            return Err(IntegrationError {
                message: "Power and session integration currently requires KDE Plasma.".into(),
            });
        }
        Ok(vec![
            dbus_item(
                "power.lock",
                "Lock Screen",
                "Lock this session",
                Bus::Session,
                "org.freedesktop.ScreenSaver",
                "/ScreenSaver",
                "org.freedesktop.ScreenSaver",
                "Lock",
                DbusArguments::None,
                Some("This locks the current session."),
                "current session",
                false,
            ),
            dbus_item(
                "power.logout",
                "Log Out",
                "End this Plasma session",
                Bus::Session,
                "org.kde.Shutdown",
                "/Shutdown",
                "org.kde.Shutdown",
                "logout",
                DbusArguments::None,
                Some("This ends the current desktop session."),
                "current session",
                false,
            ),
            dbus_item(
                "power.switch-user",
                "Switch User",
                "Open the display manager greeter",
                Bus::System,
                "org.freedesktop.DisplayManager",
                "/org/freedesktop/DisplayManager/Seat0",
                "org.freedesktop.DisplayManager.Seat",
                "SwitchToGreeter",
                DbusArguments::None,
                Some("This locks the current session and opens the user greeter."),
                "current seat",
                false,
            ),
            dbus_item(
                "power.suspend",
                "Suspend",
                "Suspend the computer",
                Bus::System,
                "org.freedesktop.login1",
                "/org/freedesktop/login1",
                "org.freedesktop.login1.Manager",
                "Suspend",
                DbusArguments::Bool(true),
                Some("This suspends the computer."),
                "this computer",
                true,
            ),
            dbus_item(
                "power.hibernate",
                "Hibernate",
                "Hibernate the computer",
                Bus::System,
                "org.freedesktop.login1",
                "/org/freedesktop/login1",
                "org.freedesktop.login1.Manager",
                "Hibernate",
                DbusArguments::Bool(true),
                Some("This hibernates the computer."),
                "this computer",
                true,
            ),
            dbus_item(
                "power.reboot",
                "Restart",
                "Restart the computer",
                Bus::System,
                "org.freedesktop.login1",
                "/org/freedesktop/login1",
                "org.freedesktop.login1.Manager",
                "Reboot",
                DbusArguments::Bool(true),
                Some("This restarts the computer and ends every session."),
                "this computer",
                true,
            ),
            dbus_item(
                "power.shutdown",
                "Shut Down",
                "Power off the computer",
                Bus::System,
                "org.freedesktop.login1",
                "/org/freedesktop/login1",
                "org.freedesktop.login1.Manager",
                "PowerOff",
                DbusArguments::Bool(true),
                Some("This powers off the computer and ends every session."),
                "this computer",
                true,
            ),
            dbus_item(
                "brightness.raise",
                "Increase Brightness",
                "Increase display brightness by 10%",
                Bus::Session,
                "org.kde.ScreenBrightness",
                "/org/kde/ScreenBrightness",
                "org.kde.ScreenBrightness",
                "AdjustBrightnessRatio",
                DbusArguments::DoubleU32(0.1, 0),
                Some("This changes display brightness by 10%."),
                "all controlled displays",
                true,
            ),
            dbus_item(
                "brightness.lower",
                "Decrease Brightness",
                "Decrease display brightness by 10%",
                Bus::Session,
                "org.kde.ScreenBrightness",
                "/org/kde/ScreenBrightness",
                "org.kde.ScreenBrightness",
                "AdjustBrightnessRatio",
                DbusArguments::DoubleU32(-0.1, 0),
                Some("This changes display brightness by 10%."),
                "all controlled displays",
                true,
            ),
        ])
    }
}

struct Processes;

impl Integration for Processes {
    fn metadata(&self) -> Metadata {
        Metadata {
            id: "processes",
            title: "Processes",
            description: "Find and terminate your own processes",
            default_priority: 64,
        }
    }

    fn search(&self, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        Ok(Vec::new())
    }

    fn answer(&self, query: &str, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        let term = query
            .trim()
            .strip_prefix("process ")
            .or_else(|| query.trim().strip_prefix("kill "))
            .map(str::trim);
        let Some(term) = term.filter(|term| !term.is_empty()) else {
            return Ok(Vec::new());
        };
        Ok(process_items(term))
    }
}

struct GlobalShortcuts;

impl Integration for GlobalShortcuts {
    fn metadata(&self) -> Metadata {
        Metadata {
            id: "global-shortcuts",
            title: "Global Shortcuts",
            description: "Trigger configured KDE shortcuts by name",
            default_priority: 65,
        }
    }

    fn search(&self, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        let path = config_home()
            .map(|path| path.join("kglobalshortcutsrc"))
            .ok_or_else(|| IntegrationError {
                message: "No KDE shortcut configuration path is available.".into(),
            })?;
        parse_global_shortcuts(&path)
    }
}

struct RecentDocuments;

impl Integration for RecentDocuments {
    fn metadata(&self) -> Metadata {
        Metadata {
            id: "recent-documents",
            title: "Recent Documents",
            description: "Files recorded by the desktop's recent-document store",
            default_priority: 21,
        }
    }

    fn search(&self, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        let path = data_home()
            .map(|path| path.join("recently-used.xbel"))
            .ok_or_else(|| IntegrationError {
                message: "No XDG data directory is available.".into(),
            })?;
        xbel_items(&path, "Recent", "recent-documents")
    }
}

struct KdePlaces;

impl Integration for KdePlaces {
    fn metadata(&self) -> Metadata {
        Metadata {
            id: "kde-places",
            title: "KDE Places",
            description: "Bookmarked, remote and removable locations",
            default_priority: 22,
        }
    }

    fn search(&self, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        let path = data_home()
            .map(|path| path.join("user-places.xbel"))
            .ok_or_else(|| IntegrationError {
                message: "No XDG data directory is available.".into(),
            })?;
        xbel_items(&path, "Place", "kde-places")
    }
}

struct Bookmarks;

impl Integration for Bookmarks {
    fn metadata(&self) -> Metadata {
        Metadata {
            id: "bookmarks",
            title: "Browser Bookmarks",
            description: "Read-only Firefox and Chromium-family bookmarks",
            default_priority: 23,
        }
    }

    fn search(&self, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        let mut items = firefox_bookmarks();
        items.extend(chromium_bookmarks());
        if items.is_empty() {
            Err(IntegrationError {
                message: "No readable Firefox or Chromium-family bookmark profile was found."
                    .into(),
            })
        } else {
            items.sort_by(|left, right| {
                left.title
                    .to_ascii_lowercase()
                    .cmp(&right.title.to_ascii_lowercase())
                    .then_with(|| left.id.cmp(&right.id))
            });
            Ok(items)
        }
    }
}

struct DeclinedCapabilities;

impl Integration for DeclinedCapabilities {
    fn metadata(&self) -> Metadata {
        Metadata {
            id: "declined",
            title: "Not provided",
            description: "Capabilities Scene deliberately declines",
            default_priority: 95,
        }
    }

    fn search(&self, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        let declined = [
            (
                "browser-data",
                "Browser tabs and history are not provided",
                "Scene reads durable bookmarks only; live tabs need a browser extension and browser history is private by default.",
                &["browser tabs", "browser history", "tabs", "history"][..],
            ),
            (
                "language",
                "Dictionary and spellcheck are not provided",
                "Dedicated language tools serve these workflows without adding network or dictionary data to the launcher.",
                &["dictionary", "definition", "spellcheck", "spelling"],
            ),
            (
                "kde-help",
                "KDE help search is not provided",
                "Scene can open help URLs, but does not maintain a separate KDE documentation index.",
                &["kde help", "documentation", "manual"],
            ),
            (
                "plasma-internals",
                "Plasma-internal actions are not provided",
                "Unstable KWin scripting and Plasma desktop internals remain outside Scene's supported contract.",
                &["kwin script", "plasma desktop action"],
            ),
            (
                "application-specific",
                "Application-specific sessions are not built in",
                "Kate sessions, Konsole profiles and Matrix rooms belong in third-party integrations.",
                &["kate session", "konsole profile", "matrix room", "neochat"],
            ),
        ];
        Ok(declined
            .into_iter()
            .map(|(id, title, explanation, keywords)| Item {
                id: format!("declined.{id}"),
                provider: String::new(),
                provider_title: String::new(),
                provider_priority: 0,
                title: title.into(),
                subtitle: explanation.into(),
                kind: Kind::Scene,
                icon: search::themed("dialog-information-symbolic"),
                category: Some("Not provided".into()),
                keywords: keywords.iter().map(|keyword| (*keyword).into()).collect(),
                action: Action::Message {
                    text: explanation.into(),
                },
                secondary_actions: Vec::new(),
            })
            .collect())
    }
}

#[derive(Clone, Copy)]
struct RunnerProvider {
    metadata: Metadata,
    service: &'static str,
    path: &'static str,
    kind: Kind,
}

impl RunnerProvider {
    fn windows() -> Self {
        Self {
            metadata: Windows.metadata(),
            service: "org.kde.KWin",
            path: "/WindowsRunner",
            kind: Kind::Application,
        }
    }

    fn files() -> Self {
        Self {
            metadata: Files.metadata(),
            service: "org.kde.runners.baloo",
            path: "/runner",
            kind: Kind::Folder,
        }
    }

    fn activities() -> Self {
        Self {
            metadata: Activities.metadata(),
            service: "org.kde.runners.activities",
            path: "/runner",
            kind: Kind::Scene,
        }
    }

    async fn query(self, service_query: &str, user_query: &str, priority: u16) -> Vec<Item> {
        let connection = match gio::bus_get_future(gio::BusType::Session).await {
            Ok(connection) => connection,
            Err(error) => return vec![runner_unavailable(self, priority, error.to_string())],
        };
        let reply = connection
            .call_future(
                Some(self.service),
                self.path,
                "org.kde.krunner1",
                "Match",
                Some(&(service_query,).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                800,
            )
            .await;
        let reply = match reply {
            Ok(reply) => reply,
            Err(error) => return vec![runner_unavailable(self, priority, error.to_string())],
        };
        let matches = reply.child_value(0);
        let mut items = matches
            .iter()
            .filter_map(|value| self.item(value, priority, user_query))
            .collect::<Vec<_>>();
        if items.is_empty()
            && self.metadata.id == "files"
            && user_query.trim_start().starts_with("file ")
        {
            items.push(answer_item(
                "files.empty",
                "No Baloo file results",
                "The existing Baloo index returned no matches; check Baloo status in System Settings.",
                Kind::Folder,
                "dialog-information-symbolic",
                user_query,
                Action::Message { text: "Scene does not build a competing file index. Enable and populate Baloo to use file search.".into() },
            ));
            let item = items.last_mut().expect("just pushed");
            item.provider = self.metadata.id.into();
            item.provider_title = self.metadata.title.into();
            item.provider_priority = priority;
        }
        items
    }

    fn item(self, value: glib::Variant, priority: u16, query: &str) -> Option<Item> {
        if value.n_children() < 6 {
            return None;
        }
        let id = value.child_value(0).str()?.to_string();
        let title = value.child_value(1).str()?.to_string();
        let icon = value.child_value(2).str().unwrap_or_default().to_string();
        let properties = glib::VariantDict::new(Some(&value.child_value(5)));
        let subtitle = properties
            .lookup_value("subtext", None)
            .and_then(|value| value.str().map(str::to_string))
            .unwrap_or_else(|| self.metadata.description.into());
        let action = Action::Dbus {
            action: DbusAction {
                id: format!("{}.run.{id}", self.metadata.id),
                title: format!("Open {title}"),
                bus: Bus::Session,
                service: self.service.into(),
                path: self.path.into(),
                interface: "org.kde.krunner1".into(),
                method: "Run".into(),
                arguments: DbusArguments::StringPair(id.clone(), String::new()),
                confirmation: None,
                observable: false,
            },
        };
        let mut item = Item {
            id: format!("{}.{}", self.metadata.id, id),
            provider: self.metadata.id.into(),
            provider_title: self.metadata.title.into(),
            provider_priority: priority,
            title,
            subtitle,
            kind: self.kind,
            icon: search::themed(if icon.is_empty() {
                self.kind.fallback_icon()
            } else {
                &icon
            }),
            category: Some(self.metadata.title.into()),
            keywords: vec![query.to_ascii_lowercase()],
            action,
            secondary_actions: Vec::new(),
        };
        if self.metadata.id == "files" && (id.starts_with("file:") || Path::new(&id).is_absolute())
        {
            let path = id.strip_prefix("file://").unwrap_or(&id).to_string();
            let parent = Path::new(&path)
                .parent()
                .map(|parent| parent.to_string_lossy().into_owned());
            if let Some(parent) = parent {
                item.secondary_actions.push(crate::search::ItemAction {
                    id: format!("files.parent.{id}"),
                    label: "Open containing folder".into(),
                    action: Action::Open { target: parent },
                });
            }
            item.secondary_actions.push(crate::search::ItemAction {
                id: format!("files.copy.{id}"),
                label: "Copy path".into(),
                action: Action::Copy {
                    text: path,
                    label: "file path".into(),
                },
            });
        }
        Some(item)
    }
}

fn runner_unavailable(runner: RunnerProvider, priority: u16, message: String) -> Item {
    let mut item = unavailable_item(runner.metadata, IntegrationError { message });
    item.provider_priority = priority;
    item
}

struct Currency;

impl Integration for Currency {
    fn metadata(&self) -> Metadata {
        Metadata {
            id: "currency",
            title: "Currency",
            description: "Currency conversion using cached ECB reference rates",
            default_priority: 1,
        }
    }

    fn search(&self, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        Ok(Vec::new())
    }
}

#[derive(Clone)]
struct CurrencyRequest {
    amount: f64,
    source: String,
    target: String,
    query: String,
}

impl CurrencyRequest {
    fn parse(query: &str) -> Option<Self> {
        let parts = query.split_whitespace().collect::<Vec<_>>();
        if parts.len() != 5
            || !parts[0].eq_ignore_ascii_case("convert")
            || !parts[3].eq_ignore_ascii_case("to")
        {
            return None;
        }
        let source = parts[2].to_ascii_uppercase();
        let target = parts[4].to_ascii_uppercase();
        if source.len() != 3
            || target.len() != 3
            || !source.chars().all(|c| c.is_ascii_alphabetic())
            || !target.chars().all(|c| c.is_ascii_alphabetic())
        {
            return None;
        }
        Some(Self {
            amount: parts[1].parse().ok()?,
            source,
            target,
            query: query.into(),
        })
    }
}

struct CurrencyAnswer {
    value: String,
    subtitle: String,
    query: String,
}

fn currency_answer(request: CurrencyRequest) -> Result<CurrencyAnswer, String> {
    let (xml, stale) = currency_rates_xml()?;
    let rates = parse_currency_rates(&xml)?;
    let source = rates
        .get(&request.source)
        .copied()
        .ok_or_else(|| format!("ECB does not publish a {} reference rate.", request.source))?;
    let target = rates
        .get(&request.target)
        .copied()
        .ok_or_else(|| format!("ECB does not publish a {} reference rate.", request.target))?;
    let converted = request.amount / source * target;
    Ok(CurrencyAnswer {
        value: format!("{converted:.4} {}", request.target),
        subtitle: format!(
            "{} {} · ECB reference rates{}",
            request.amount,
            request.source,
            if stale { " · cached" } else { "" }
        ),
        query: request.query,
    })
}

fn currency_rates_xml() -> Result<(String, bool), String> {
    let path = cache_home().map(|path| path.join("scene").join("eurofxref-daily.xml"));
    if let Some(path) = path.as_ref()
        && let Ok(metadata) = std::fs::metadata(path)
        && let Ok(modified) = metadata.modified()
        && modified
            .elapsed()
            .is_ok_and(|age| age < Duration::from_secs(12 * 60 * 60))
        && let Ok(xml) = std::fs::read_to_string(path)
    {
        return Ok((xml, false));
    }
    let fetched = ureq::get("https://www.ecb.europa.eu/stats/eurofxref/eurofxref-daily.xml")
        .config()
        .timeout_global(Some(Duration::from_secs(2)))
        .build()
        .call()
        .and_then(|response| response.into_body().read_to_string());
    match fetched {
        Ok(xml) => {
            if let Some(path) = path {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(path, &xml);
            }
            Ok((xml, false))
        }
        Err(error) => {
            if let Some(path) = path
                && let Ok(xml) = std::fs::read_to_string(path)
            {
                return Ok((xml, true));
            }
            Err(format!("ECB rates could not be loaded: {error}"))
        }
    }
}

fn parse_currency_rates(xml: &str) -> Result<BTreeMap<String, f64>, String> {
    let mut rates = BTreeMap::from([("EUR".to_string(), 1.0)]);
    let mut reader = quick_xml::Reader::from_str(xml);
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Empty(element)) if element.name().as_ref() == b"Cube" => {
                let mut currency = None;
                let mut rate = None;
                for attribute in element.attributes().flatten() {
                    let value = attribute
                        .decode_and_unescape_value(reader.decoder())
                        .map(|value| value.into_owned())
                        .ok();
                    match attribute.key.as_ref() {
                        b"currency" => currency = value,
                        b"rate" => rate = value.and_then(|value| value.parse::<f64>().ok()),
                        _ => {}
                    }
                }
                if let (Some(currency), Some(rate)) = (currency, rate) {
                    rates.insert(currency, rate);
                }
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Err(error) => return Err(format!("ECB rate data is malformed: {error}")),
            _ => {}
        }
    }
    (rates.len() > 1)
        .then_some(rates)
        .ok_or_else(|| "ECB rate data contained no currencies.".into())
}

fn cache_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
}

struct Calculator;

impl Integration for Calculator {
    fn metadata(&self) -> Metadata {
        Metadata {
            id: "calculator",
            title: "Calculator and units",
            description: "Arithmetic and local unit conversion",
            default_priority: 0,
        }
    }

    fn search(&self, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        Ok(Vec::new())
    }

    fn answer(&self, query: &str, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        let trimmed = query.trim();
        let expression = trimmed
            .strip_prefix('=')
            .or_else(|| trimmed.strip_prefix("calc "))
            .or_else(|| trimmed.strip_prefix("calculate "))
            .or_else(|| trimmed.strip_prefix("convert "));
        let Some(expression) = expression.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(Vec::new());
        };
        if CurrencyRequest::parse(trimmed).is_some() {
            return Ok(Vec::new());
        }
        let mut context = fend_core::Context::new();
        match fend_core::evaluate(expression, &mut context) {
            Ok(result) => {
                let answer = result.get_main_result().to_string();
                Ok(vec![answer_item(
                    "calculator.answer",
                    &answer,
                    &format!("Calculator · {expression}"),
                    Kind::Scene,
                    "accessories-calculator-symbolic",
                    query,
                    Action::Copy {
                        text: answer.clone(),
                        label: "answer".into(),
                    },
                )])
            }
            Err(error) => Ok(vec![answer_item(
                "calculator.error",
                "Calculator could not answer",
                &error.to_string(),
                Kind::Scene,
                "dialog-warning-symbolic",
                query,
                Action::Message {
                    text: error.to_string(),
                },
            )]),
        }
    }
}

struct DateAndTime;

impl Integration for DateAndTime {
    fn metadata(&self) -> Metadata {
        Metadata {
            id: "date-time",
            title: "Date and time",
            description: "Current time and named timezone conversion",
            default_priority: 2,
        }
    }

    fn search(&self, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        Ok(Vec::new())
    }

    fn answer(&self, query: &str, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        let trimmed = query.trim();
        if trimmed.eq_ignore_ascii_case("time") || trimmed.eq_ignore_ascii_case("date") {
            let answer = Local::now()
                .format("%A, %B %-d, %Y · %-I:%M:%S %p %Z")
                .to_string();
            return Ok(vec![copy_answer(
                "date-time.local",
                &answer,
                "Local date and time",
                query,
            )]);
        }
        let Some(request) = trimmed.strip_prefix("time ").map(str::trim) else {
            return Ok(Vec::new());
        };
        if let Ok(zone) = request.parse::<Tz>() {
            let answer = Utc::now()
                .with_timezone(&zone)
                .format("%Y-%m-%d %-I:%M:%S %p %Z")
                .to_string();
            return Ok(vec![copy_answer("date-time.zone", &answer, request, query)]);
        }
        let parts = request.split_whitespace().collect::<Vec<_>>();
        if parts.len() == 5 && parts[3].eq_ignore_ascii_case("to") {
            let parsed = NaiveDateTime::parse_from_str(
                &format!("{} {}", parts[0], parts[1]),
                "%Y-%m-%d %H:%M",
            );
            let source = parts[2].parse::<Tz>();
            let target = parts[4].parse::<Tz>();
            if let (Ok(local), Ok(source), Ok(target)) = (parsed, source, target)
                && let Some(moment) = source.from_local_datetime(&local).single()
            {
                let answer = moment
                    .with_timezone(&target)
                    .format("%Y-%m-%d %-I:%M %p %Z")
                    .to_string();
                return Ok(vec![copy_answer(
                    "date-time.convert",
                    &answer,
                    parts[4],
                    query,
                )]);
            }
        }
        Ok(vec![answer_item(
            "date-time.syntax",
            "Time query not understood",
            "Use “time Europe/London” or “time 2026-08-24 14:30 America/Chicago to Europe/London”.",
            Kind::Scene,
            "dialog-information-symbolic",
            query,
            Action::Message {
                text: "Use an IANA timezone name, for example Europe/London or America/Chicago."
                    .into(),
            },
        )])
    }
}

struct Colors;

impl Integration for Colors {
    fn metadata(&self) -> Metadata {
        Metadata {
            id: "colors",
            title: "Colors",
            description: "Convert CSS colors between hex, RGB and HSL",
            default_priority: 3,
        }
    }

    fn search(&self, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        Ok(Vec::new())
    }

    fn answer(&self, query: &str, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        let Some(value) = query.trim().strip_prefix("color ").map(str::trim) else {
            return Ok(Vec::new());
        };
        let color = match csscolorparser::parse(value) {
            Ok(color) => color,
            Err(error) => {
                return Ok(vec![answer_item(
                    "colors.error",
                    "Color not understood",
                    &error.to_string(),
                    Kind::Scene,
                    "dialog-warning-symbolic",
                    query,
                    Action::Message {
                        text: error.to_string(),
                    },
                )]);
            }
        };
        let hex = color.to_css_hex();
        let rgb = color.to_css_rgb();
        let hsl = color.to_css_hsl();
        let mut item = answer_item(
            "colors.answer",
            &hex,
            &format!("{rgb} · {hsl}"),
            Kind::Scene,
            "applications-graphics-symbolic",
            query,
            Action::Copy {
                text: hex.clone(),
                label: "hex color".into(),
            },
        );
        item.secondary_actions = vec![
            crate::search::ItemAction {
                id: "colors.copy-rgb".into(),
                label: format!("Copy {rgb}"),
                action: Action::Copy {
                    text: rgb,
                    label: "RGB color".into(),
                },
            },
            crate::search::ItemAction {
                id: "colors.copy-hsl".into(),
                label: format!("Copy {hsl}"),
                action: Action::Copy {
                    text: hsl,
                    label: "HSL color".into(),
                },
            },
        ];
        Ok(vec![item])
    }
}

struct Characters;

impl Integration for Characters {
    fn metadata(&self) -> Metadata {
        Metadata {
            id: "characters",
            title: "Characters",
            description: "Unicode lookup by code point or name",
            default_priority: 4,
        }
    }

    fn search(&self, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        Ok(Vec::new())
    }

    fn answer(&self, query: &str, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        let Some(request) = query.trim().strip_prefix("char ").map(str::trim) else {
            return Ok(Vec::new());
        };
        let character = request
            .strip_prefix("U+")
            .or_else(|| request.strip_prefix("u+"))
            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
            .and_then(char::from_u32)
            .or_else(|| unicode_names2::character(&request.to_ascii_uppercase()));
        let Some(character) = character else {
            return Ok(vec![answer_item(
                "characters.error",
                "Character not found",
                "Use a Unicode name or code point such as U+1F680.",
                Kind::Scene,
                "dialog-warning-symbolic",
                query,
                Action::Message {
                    text: "No Unicode character matched that name or code point.".into(),
                },
            )]);
        };
        let name = unicode_names2::name(character)
            .map(|name| name.to_string())
            .unwrap_or_else(|| "Unnamed character".into());
        Ok(vec![answer_item(
            "characters.answer",
            &character.to_string(),
            &format!("U+{:04X} · {name}", character as u32),
            Kind::Scene,
            "accessories-character-map-symbolic",
            query,
            Action::Copy {
                text: character.to_string(),
                label: "character".into(),
            },
        )])
    }
}

struct Commands;

impl Integration for Commands {
    fn metadata(&self) -> Metadata {
        Metadata {
            id: "commands",
            title: "Commands",
            description: "Explicit bounded command execution",
            default_priority: 5,
        }
    }

    fn search(&self, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        Ok(Vec::new())
    }

    fn answer(&self, query: &str, config: &Config) -> Result<Vec<Item>, IntegrationError> {
        let trimmed = query.trim();
        let Some(command) = trimmed.strip_prefix("run") else {
            return Ok(Vec::new());
        };
        if !command.is_empty() && !command.starts_with(char::is_whitespace) {
            return Ok(Vec::new());
        }
        let command = command.trim();
        if command.is_empty() {
            if !config.command_history_enabled {
                return Ok(vec![answer_item(
                    "commands.history-off",
                    "Command history is off",
                    "Enable it in Scene Settings; explicit commands still work.",
                    Kind::Scene,
                    "utilities-terminal-symbolic",
                    query,
                    Action::ShowSettings,
                )]);
            }
            return Ok(crate::actions::command_history()
                .into_iter()
                .enumerate()
                .filter_map(|(position, entry)| command_item(&entry, config, position).ok())
                .collect());
        }
        command_item(command, config, 0)
            .map(|item| vec![item])
            .or_else(|error| {
                Ok(vec![answer_item(
                    "commands.error",
                    "Command not available",
                    &error.message,
                    Kind::Scene,
                    "dialog-warning-symbolic",
                    query,
                    Action::Message {
                        text: error.message.clone(),
                    },
                )])
            })
    }
}

struct WebShortcuts;

impl Integration for WebShortcuts {
    fn metadata(&self) -> Metadata {
        Metadata {
            id: "web-shortcuts",
            title: "Web shortcuts",
            description: "KDE keyworded web searches",
            default_priority: 41,
        }
    }

    fn search(&self, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        Ok(Vec::new())
    }

    fn answer(&self, query: &str, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        let trimmed = query.trim();
        for shortcut in web_shortcuts() {
            for key in &shortcut.keys {
                let term = trimmed
                    .strip_prefix(&format!("{key}:"))
                    .or_else(|| trimmed.strip_prefix(&format!("{key} ")))
                    .map(str::trim)
                    .filter(|term| !term.is_empty());
                if let Some(term) = term {
                    let encoded = utf8_percent_encode(term, NON_ALPHANUMERIC).to_string();
                    let url = shortcut
                        .query
                        .replace("\\{@}", &encoded)
                        .replace("{@}", &encoded);
                    return Ok(vec![answer_item(
                        &format!("web-shortcuts.{}", key),
                        &format!("Search {} for {term}", shortcut.name),
                        &url,
                        Kind::Web,
                        &shortcut.icon,
                        query,
                        Action::Open {
                            target: url.clone(),
                        },
                    )]);
                }
            }
        }
        Ok(Vec::new())
    }
}

fn copy_answer(id: &str, answer: &str, subtitle: &str, query: &str) -> Item {
    answer_item(
        id,
        answer,
        subtitle,
        Kind::Scene,
        "edit-copy-symbolic",
        query,
        Action::Copy {
            text: answer.into(),
            label: "answer".into(),
        },
    )
}

fn answer_item(
    id: &str,
    title: &str,
    subtitle: &str,
    kind: Kind,
    icon: &str,
    query: &str,
    action: Action,
) -> Item {
    Item {
        id: id.into(),
        provider: String::new(),
        provider_title: String::new(),
        provider_priority: 0,
        title: title.into(),
        subtitle: subtitle.into(),
        kind,
        icon: search::themed(icon),
        category: Some("Answer".into()),
        keywords: vec![query.trim().to_ascii_lowercase()],
        action,
        secondary_actions: Vec::new(),
    }
}

fn command_item(command: &str, config: &Config, position: usize) -> Result<Item, IntegrationError> {
    if command.contains(['\n', '\r', '\0']) {
        return Err(IntegrationError {
            message: "Commands cannot contain line breaks or NUL bytes.".into(),
        });
    }
    let argv = glib::shell_parse_argv(command).map_err(|error| IntegrationError {
        message: error.to_string(),
    })?;
    let mut argv = argv
        .into_iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let executable = argv
        .first()
        .ok_or_else(|| IntegrationError {
            message: "Name an executable after run.".into(),
        })?
        .clone();
    let resolved = system::locate(&executable).ok_or_else(|| IntegrationError {
        message: format!("{executable} is not installed or not on PATH."),
    })?;
    argv.remove(0);
    let display = serde_json::to_string(
        &std::iter::once(resolved.to_string_lossy().into_owned())
            .chain(argv.iter().cloned())
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| command.into());
    let spec = CommandSpec::read_only(resolved.to_string_lossy(), argv)
        .with_working_directory(config.directory.path.to_string_lossy())
        .with_timeout(Duration::from_secs(30))
        .with_output_limit(64 * 1024);
    let mut action = ProcessAction::mutating(
        format!("commands.run.{position}"),
        format!("Run {executable}"),
        spec,
        Confirmation {
            summary: "This explicitly runs a command without a shell.".into(),
            target: display.clone(),
        },
    );
    if config.command_history_enabled {
        action = action.with_history(command);
    }
    let mut item = answer_item(
        &format!("commands.run.{position}"),
        &format!("Run {executable}"),
        &format!(
            "argv {display} · cwd {} · 30 s · 64 KiB · inherited environment",
            config.directory.path.display()
        ),
        Kind::Scene,
        "utilities-terminal-symbolic",
        command,
        Action::Process { action },
    );
    item.secondary_actions.push(crate::search::ItemAction {
        id: format!("commands.copy.{position}"),
        label: "Copy command text".into(),
        action: Action::Copy {
            text: command.into(),
            label: "command".into(),
        },
    });
    Ok(item)
}

struct WebShortcut {
    name: String,
    icon: String,
    keys: Vec<String>,
    query: String,
}

fn web_shortcuts() -> Vec<WebShortcut> {
    let mut shortcuts = Vec::new();
    let roots = [
        PathBuf::from("/usr/share/kf6/searchproviders"),
        PathBuf::from("/usr/share/kservices6/searchproviders"),
    ];
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let file = glib::KeyFile::new();
            if file
                .load_from_file(entry.path(), glib::KeyFileFlags::NONE)
                .is_err()
            {
                continue;
            }
            const GROUP: &str = "Desktop Entry";
            let (Ok(name), Ok(keys), Ok(query)) = (
                file.locale_string(GROUP, "Name", None),
                file.string_list(GROUP, "Keys"),
                file.string(GROUP, "Query"),
            ) else {
                continue;
            };
            shortcuts.push(WebShortcut {
                name: name.into(),
                icon: file
                    .string(GROUP, "Icon")
                    .map(Into::into)
                    .unwrap_or_else(|_| "web-browser-symbolic".into()),
                keys: keys.iter().map(|key| key.to_string()).collect(),
                query: query.into(),
            });
        }
    }
    shortcuts
}

#[allow(clippy::too_many_arguments)]
fn dbus_item(
    id: &str,
    title: &str,
    subtitle: &str,
    bus: Bus,
    service: &str,
    path: &str,
    interface: &str,
    method: &str,
    arguments: DbusArguments,
    confirmation: Option<&str>,
    target: &str,
    observable: bool,
) -> Item {
    Item {
        id: id.into(),
        provider: String::new(),
        provider_title: String::new(),
        provider_priority: 0,
        title: title.into(),
        subtitle: subtitle.into(),
        kind: Kind::Scene,
        icon: search::themed("system-run-symbolic"),
        category: Some("System".into()),
        keywords: vec![title.to_ascii_lowercase(), subtitle.to_ascii_lowercase()],
        action: Action::Dbus {
            action: DbusAction {
                id: id.into(),
                title: title.into(),
                bus,
                service: service.into(),
                path: path.into(),
                interface: interface.into(),
                method: method.into(),
                arguments,
                confirmation: confirmation.map(|summary| Confirmation {
                    summary: summary.into(),
                    target: target.into(),
                }),
                observable,
            },
        },
        secondary_actions: Vec::new(),
    }
}

fn process_items(term: &str) -> Vec<Item> {
    let current_uid = read_uid("/proc/self/status");
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut items = Vec::new();
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        if pid <= 1 || pid == std::process::id() {
            continue;
        }
        let status_path = entry.path().join("status");
        if read_uid(&status_path) != current_uid {
            continue;
        }
        let name = std::fs::read_to_string(&status_path)
            .ok()
            .and_then(|status| {
                status
                    .lines()
                    .find_map(|line| line.strip_prefix("Name:\t").map(str::to_string))
            })
            .unwrap_or_default();
        if name.is_empty()
            || !name
                .to_ascii_lowercase()
                .contains(&term.to_ascii_lowercase())
        {
            continue;
        }
        let confirmation = Confirmation {
            summary: format!("This sends SIGTERM to {name}."),
            target: format!("{name}, PID {pid}"),
        };
        let mut item = Item {
            id: format!("processes.{pid}"),
            provider: String::new(),
            provider_title: String::new(),
            provider_priority: 0,
            title: name.clone(),
            subtitle: format!("PID {pid} · owned by the current user"),
            kind: Kind::Scene,
            icon: search::themed("utilities-system-monitor-symbolic"),
            category: Some("Process".into()),
            keywords: vec![term.into(), pid.to_string()],
            action: Action::Signal {
                action: SignalAction {
                    id: format!("processes.term.{pid}"),
                    title: format!("Terminate {name}"),
                    pid,
                    signal: 15,
                    confirmation,
                },
            },
            secondary_actions: Vec::new(),
        };
        item.secondary_actions.push(crate::search::ItemAction {
            id: format!("processes.kill.{pid}"),
            label: format!("Force stop {name} (SIGKILL)"),
            action: Action::Signal {
                action: SignalAction {
                    id: format!("processes.kill.{pid}"),
                    title: format!("Force stop {name}"),
                    pid,
                    signal: 9,
                    confirmation: Confirmation {
                        summary: format!("This force-stops {name} without allowing cleanup."),
                        target: format!("{name}, PID {pid}"),
                    },
                },
            },
        });
        items.push(item);
        if items.len() == 30 {
            break;
        }
    }
    items.sort_by(|left, right| {
        left.title
            .to_ascii_lowercase()
            .cmp(&right.title.to_ascii_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
    items
}

fn read_uid(path: impl AsRef<Path>) -> Option<u32> {
    std::fs::read_to_string(path)
        .ok()?
        .lines()
        .find_map(|line| {
            line.strip_prefix("Uid:")
                .and_then(|value| value.split_whitespace().next())?
                .parse()
                .ok()
        })
}

fn parse_global_shortcuts(path: &Path) -> Result<Vec<Item>, IntegrationError> {
    let file = glib::KeyFile::new();
    file.load_from_file(path, glib::KeyFileFlags::NONE)
        .map_err(|error| IntegrationError {
            message: format!("KDE shortcut configuration could not be read: {error}"),
        })?;
    let mut items = Vec::new();
    for group in file.groups() {
        let group = group.to_string();
        let friendly = file
            .string(&group, "_k_friendly_name")
            .map(|value| value.to_string())
            .unwrap_or_else(|_| group.clone());
        let Ok(keys) = file.keys(&group) else {
            continue;
        };
        for key in keys
            .iter()
            .map(|key| key.to_string())
            .filter(|key| key != "_k_friendly_name")
        {
            let Ok(value) = file.string(&group, &key) else {
                continue;
            };
            let fields = value.splitn(3, ',').collect::<Vec<_>>();
            let active = fields.first().copied().unwrap_or_default();
            if active.is_empty() || active.eq_ignore_ascii_case("none") {
                continue;
            }
            let title = fields
                .get(2)
                .copied()
                .filter(|value| !value.is_empty())
                .unwrap_or(&key);
            let component = component_path(&group);
            items.push(Item {
                id: format!(
                    "global-shortcuts.{}.{}",
                    component.trim_start_matches("/component/"),
                    key
                ),
                provider: String::new(),
                provider_title: String::new(),
                provider_priority: 0,
                title: title.into(),
                subtitle: format!("{friendly} · {}", active.replace("\\t", ", ")),
                kind: Kind::Scene,
                icon: search::themed("preferences-desktop-keyboard-shortcuts"),
                category: Some("Shortcut".into()),
                keywords: vec![
                    friendly.to_ascii_lowercase(),
                    key.to_ascii_lowercase(),
                    active.to_ascii_lowercase(),
                ],
                action: Action::Dbus {
                    action: DbusAction {
                        id: format!("global-shortcuts.invoke.{key}"),
                        title: title.into(),
                        bus: Bus::Session,
                        service: "org.kde.kglobalaccel".into(),
                        path: component,
                        interface: "org.kde.kglobalaccel.Component".into(),
                        method: "invokeShortcut".into(),
                        arguments: DbusArguments::String(key.clone()),
                        confirmation: Some(Confirmation {
                            summary: "This invokes the configured KDE global action.".into(),
                            target: format!("{friendly}: {title}"),
                        }),
                        observable: false,
                    },
                },
                secondary_actions: Vec::new(),
            });
        }
    }
    Ok(items)
}

fn component_path(group: &str) -> String {
    let component = group
        .strip_prefix("services][")
        .and_then(|value| value.strip_suffix(".desktop"))
        .unwrap_or(group);
    let normalized = component
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("/component/{normalized}")
}

fn config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
}

fn data_home() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("share"))
        })
}

fn xbel_items(path: &Path, category: &str, provider: &str) -> Result<Vec<Item>, IntegrationError> {
    let bookmarks = xbel_entries(path)?;
    let mut items = Vec::new();
    for (uri, recorded_title) in bookmarks.into_iter().take(500) {
        let file = gio::File::for_uri(&uri);
        let local_path = file.path();
        let title = recorded_title.unwrap_or_else(|| {
            local_path
                .as_ref()
                .and_then(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| uri.clone())
        });
        let availability = match local_path.as_ref() {
            Some(path) if path.exists() => path.display().to_string(),
            Some(path) => format!("Unavailable · {}", path.display()),
            None => format!("Remote location · {uri}"),
        };
        let target = local_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| uri.clone());
        let mut item = Item {
            id: format!("{provider}.{:016x}", stable_id(&uri)),
            provider: String::new(),
            provider_title: String::new(),
            provider_priority: 0,
            title,
            subtitle: availability,
            kind: Kind::Folder,
            icon: search::themed(if category == "Recent" {
                "document-open-recent-symbolic"
            } else {
                "folder-bookmark-symbolic"
            }),
            category: Some(category.into()),
            keywords: vec![uri.to_ascii_lowercase(), target.to_ascii_lowercase()],
            action: Action::Open {
                target: target.clone(),
            },
            secondary_actions: Vec::new(),
        };
        if let Some(parent) = local_path.as_ref().and_then(|path| path.parent()) {
            item.secondary_actions.push(crate::search::ItemAction {
                id: format!("{provider}.parent.{:016x}", stable_id(&uri)),
                label: "Open containing folder".into(),
                action: Action::Open {
                    target: parent.to_string_lossy().into_owned(),
                },
            });
        }
        item.secondary_actions.push(crate::search::ItemAction {
            id: format!("{provider}.copy.{:016x}", stable_id(&uri)),
            label: "Copy path or URI".into(),
            action: Action::Copy {
                text: target,
                label: "path or URI".into(),
            },
        });
        items.push(item);
    }
    Ok(items)
}

fn xbel_entries(path: &Path) -> Result<Vec<(String, Option<String>)>, IntegrationError> {
    let xml = std::fs::read_to_string(path).map_err(|error| IntegrationError {
        message: format!("{} could not be read: {error}", path.display()),
    })?;
    let mut reader = quick_xml::Reader::from_str(&xml);
    let mut entries = Vec::new();
    let mut bookmark = None;
    let mut reading_title = false;
    let mut title = String::new();
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(element)) => {
                let name = element.name();
                let local = name.as_ref().rsplit(|byte| *byte == b':').next();
                match local {
                    Some(b"bookmark") => {
                        bookmark = element.attributes().flatten().find_map(|attribute| {
                            (attribute.key.as_ref() == b"href")
                                .then(|| attribute.decode_and_unescape_value(reader.decoder()).ok())
                                .flatten()
                                .map(|value| value.into_owned())
                        });
                        title.clear();
                    }
                    Some(b"title") if bookmark.is_some() => reading_title = true,
                    _ => {}
                }
            }
            Ok(quick_xml::events::Event::Empty(element)) => {
                let name = element.name();
                let local = name.as_ref().rsplit(|byte| *byte == b':').next();
                if local == Some(b"bookmark")
                    && let Some(uri) = element.attributes().flatten().find_map(|attribute| {
                        (attribute.key.as_ref() == b"href")
                            .then(|| attribute.decode_and_unescape_value(reader.decoder()).ok())
                            .flatten()
                            .map(|value| value.into_owned())
                    })
                {
                    entries.push((uri, None));
                }
            }
            Ok(quick_xml::events::Event::Text(text)) if reading_title => {
                if let Ok(text) = text.decode() {
                    title.push_str(&text);
                }
            }
            Ok(quick_xml::events::Event::End(element)) => {
                let name = element.name();
                let local = name.as_ref().rsplit(|byte| *byte == b':').next();
                match local {
                    Some(b"title") => reading_title = false,
                    Some(b"bookmark") => {
                        if let Some(uri) = bookmark.take() {
                            entries.push((
                                uri,
                                (!title.trim().is_empty()).then(|| title.trim().to_string()),
                            ));
                        }
                        title.clear();
                    }
                    _ => {}
                }
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Err(error) => {
                return Err(IntegrationError {
                    message: format!("{} contains malformed XBEL: {error}", path.display()),
                });
            }
            _ => {}
        }
    }
    Ok(entries)
}

fn firefox_bookmarks() -> Vec<Item> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let root = home.join(".mozilla/firefox");
    let Ok(profiles) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    for profile in profiles.flatten() {
        let database = profile.path().join("places.sqlite");
        if !database.is_file() {
            continue;
        }
        let profile_name = profile.file_name().to_string_lossy().into_owned();
        let Ok(connection) = rusqlite::Connection::open_with_flags(
            &database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) else {
            continue;
        };
        let Ok(mut statement) = connection.prepare("SELECT COALESCE(b.title, p.title, p.url), p.url FROM moz_bookmarks b JOIN moz_places p ON p.id = b.fk WHERE b.type = 1 AND p.url IS NOT NULL ORDER BY b.dateAdded DESC LIMIT 500") else { continue };
        let Ok(rows) = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }) else {
            continue;
        };
        for row in rows.flatten() {
            items.push(browser_bookmark("firefox", &profile_name, row.0, row.1));
        }
    }
    items
}

fn chromium_bookmarks() -> Vec<Item> {
    let Some(config) = config_home() else {
        return Vec::new();
    };
    let roots = ["chromium", "google-chrome", "BraveSoftware/Brave-Browser"];
    let mut items = Vec::new();
    for root in roots {
        let root = config.join(root);
        let Ok(profiles) = std::fs::read_dir(&root) else {
            continue;
        };
        for profile in profiles.flatten() {
            let path = profile.path().join("Bookmarks");
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            let Ok(document) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            let profile_name = profile.file_name().to_string_lossy().into_owned();
            if let Some(roots) = document.get("roots").and_then(serde_json::Value::as_object) {
                for value in roots.values() {
                    chromium_nodes(
                        value,
                        root.file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("Chromium"),
                        &profile_name,
                        &mut items,
                    );
                }
            }
        }
    }
    items
}

fn chromium_nodes(value: &serde_json::Value, browser: &str, profile: &str, items: &mut Vec<Item>) {
    if items.len() >= 2_000 {
        return;
    }
    if value.get("type").and_then(serde_json::Value::as_str) == Some("url")
        && let (Some(name), Some(url)) = (
            value.get("name").and_then(serde_json::Value::as_str),
            value.get("url").and_then(serde_json::Value::as_str),
        )
    {
        items.push(browser_bookmark(browser, profile, name.into(), url.into()));
    }
    if let Some(children) = value.get("children").and_then(serde_json::Value::as_array) {
        for child in children {
            chromium_nodes(child, browser, profile, items);
        }
    }
}

fn browser_bookmark(browser: &str, profile: &str, title: String, url: String) -> Item {
    let id = stable_id(&format!("{browser}\0{profile}\0{url}"));
    Item {
        id: format!("bookmarks.{id:016x}"),
        provider: String::new(),
        provider_title: String::new(),
        provider_priority: 0,
        title,
        subtitle: format!("{browser} · {profile} · {url}"),
        kind: Kind::Web,
        icon: search::themed("bookmarks-symbolic"),
        category: Some("Bookmark".into()),
        keywords: vec![
            browser.to_ascii_lowercase(),
            profile.to_ascii_lowercase(),
            url.to_ascii_lowercase(),
        ],
        action: Action::Open {
            target: url.clone(),
        },
        secondary_actions: vec![crate::search::ItemAction {
            id: format!("bookmarks.copy.{id:016x}"),
            label: "Copy URL".into(),
            action: Action::Copy {
                text: url,
                label: "bookmark URL".into(),
            },
        }],
    }
}

fn stable_id(value: &str) -> u64 {
    value
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
}

struct Applications;

struct BuiltinPlaces;

impl Integration for BuiltinPlaces {
    fn metadata(&self) -> Metadata {
        Metadata {
            id: "places",
            title: "Places",
            description: "Built-in common folders",
            default_priority: 24,
        }
    }

    fn search(&self, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        Ok(search::catalogue()
            .into_iter()
            .filter(|item| item.provider == "places")
            .collect())
    }
}

struct Documentation;

impl Integration for Documentation {
    fn metadata(&self) -> Metadata {
        Metadata {
            id: "web",
            title: "Documentation",
            description: "Built-in documentation links",
            default_priority: 40,
        }
    }

    fn search(&self, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        Ok(search::catalogue()
            .into_iter()
            .filter(|item| item.provider == "web")
            .collect())
    }
}

struct SceneCommands;

impl Integration for SceneCommands {
    fn metadata(&self) -> Metadata {
        Metadata {
            id: "scene",
            title: "Scene",
            description: "Scene settings and information",
            default_priority: 90,
        }
    }

    fn search(&self, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        Ok(search::catalogue()
            .into_iter()
            .filter(|item| item.provider == "scene")
            .collect())
    }
}

impl Integration for Applications {
    fn metadata(&self) -> Metadata {
        Metadata {
            id: "applications",
            title: "Applications",
            description: "Installed desktop applications",
            default_priority: 10,
        }
    }

    fn search(&self, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        Ok(crate::apps::installed())
    }
}

struct Terminal;

impl Integration for Terminal {
    fn metadata(&self) -> Metadata {
        Metadata {
            id: "terminal",
            title: "Terminal",
            description: "Open an installed terminal",
            default_priority: 60,
        }
    }

    fn search(&self, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        let program = ["konsole", "kgx", "gnome-terminal", "foot", "xterm"]
            .into_iter()
            .find(|program| system::executable_on_path(program))
            .ok_or_else(|| IntegrationError { message: "No supported terminal is installed (Konsole, Console, GNOME Terminal, Foot, or xterm).".into() })?;
        Ok(vec![process_item(
            "terminal.open",
            "Open Terminal",
            format!("Open {program}"),
            "Terminal",
            &["shell", "console", "command line"],
            ProcessAction::detached(
                "terminal.open",
                "Open Terminal",
                CommandSpec::read_only(program, [] as [&str; 0]),
            ),
        )])
    }
}

struct SystemInformation;

impl Integration for SystemInformation {
    fn metadata(&self) -> Metadata {
        Metadata {
            id: "system-information",
            title: "System information",
            description: "Read-only operating system details",
            default_priority: 61,
        }
    }

    fn search(&self, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        if !system::executable_on_path("uname") {
            return Err(IntegrationError {
                message: "uname is not installed or not on PATH.".into(),
            });
        }
        Ok(vec![process_item(
            "system.information",
            "Show System Information",
            "Read operating system and kernel details",
            "System",
            &["kernel", "os", "uname", "version"],
            ProcessAction::read_only(
                "system.information",
                "Show System Information",
                CommandSpec::read_only("uname", ["-a"]),
            ),
        )])
    }
}

struct ConfiguredDirectory;

impl Integration for ConfiguredDirectory {
    fn metadata(&self) -> Metadata {
        Metadata {
            id: "directory",
            title: "Configured directory",
            description: "Open the configured directory",
            default_priority: 25,
        }
    }

    fn search(&self, config: &Config) -> Result<Vec<Item>, IntegrationError> {
        let path = config.directory.path.to_string_lossy().into_owned();
        Ok(vec![Item {
            id: "directory.configured".into(),
            provider: String::new(),
            provider_title: String::new(),
            provider_priority: 0,
            title: "Open Configured Directory".into(),
            subtitle: path.clone(),
            kind: Kind::Folder,
            icon: search::themed("folder"),
            category: Some("Directory".into()),
            keywords: vec!["folder".into(), "files".into(), "scene_directory".into()],
            action: Action::Open { target: path },
            secondary_actions: Vec::new(),
        }])
    }
}

/// Package capabilities, through whichever distribution adapter this machine
/// actually has. See `packages` for the detection rule and the commands.
struct Packages;

/// The keywords that turn a query into package work, and what each asks for.
/// They live here rather than in `ui`, so the launcher surface still knows
/// nothing about any particular provider.
const PACKAGE_KEYWORDS: &[(&str, &[Capability])] = &[
    (
        "pkg",
        &[
            Capability::Search,
            Capability::Metadata,
            Capability::Installed,
        ],
    ),
    (
        "package",
        &[
            Capability::Search,
            Capability::Metadata,
            Capability::Installed,
        ],
    ),
    ("install", &[Capability::Install]),
    ("remove", &[Capability::Remove]),
    ("uninstall", &[Capability::Remove]),
];

impl Integration for Packages {
    fn metadata(&self) -> Metadata {
        Metadata {
            id: "packages",
            title: "Packages",
            description: "Search, inspect, install and remove packages",
            default_priority: 50,
        }
    }

    fn search(&self, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        let detected = detected_adapter()?;
        let available = detected.capabilities();
        let capability_list = if available.is_empty() {
            "no capabilities".to_string()
        } else {
            available
                .iter()
                .map(|capability| capability.label())
                .collect::<Vec<_>>()
                .join(", ")
        };

        let mut items = vec![Item {
            id: "packages.keywords".into(),
            provider: String::new(),
            provider_title: String::new(),
            provider_priority: 0,
            title: "Package Commands".into(),
            subtitle: "Type pkg, install, or remove followed by a package name".into(),
            kind: Kind::Scene,
            icon: search::themed("package-x-generic"),
            category: Some("Packages".into()),
            keywords: vec![
                "package".into(),
                "packages".into(),
                "install".into(),
                "remove".into(),
                "uninstall".into(),
                "apt".into(),
                "dnf".into(),
                "pacman".into(),
            ],
            action: Action::Message {
                text: format!(
                    "{} tooling detected. Available: {capability_list}. Type “pkg name”, “install name”, or “remove name”.",
                    detected.family().label()
                ),
            },
            secondary_actions: Vec::new(),
        }];

        // The one package question that names no package, so it belongs in
        // the static index rather than in an answer.
        items.push(match detected.plan(Capability::Updates, None) {
            Ok(plan) => process_item(
                "packages.updates",
                "Check for Package Updates",
                plan.display.clone(),
                "Packages",
                &["update", "upgrade", "updates", "packages"],
                ProcessAction::read_only(
                    "packages.updates",
                    "Check for Package Updates",
                    plan.spec,
                ),
            ),
            Err(unsupported) => unsupported_item(
                "packages.updates",
                "Check for Package Updates",
                &unsupported,
                &["update", "upgrade", "updates", "packages"],
            ),
        });

        Ok(items)
    }

    fn answer(&self, query: &str, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        if package_status_term(query).is_some() {
            return Ok(Vec::new());
        }
        let Some((capabilities, text)) = package_keyword(query) else {
            return Ok(Vec::new());
        };

        // Detection is re-run per query rather than cached, so a package
        // manager installed while Scene is resident is picked up the same way
        // a newly installed application is. It costs a handful of `stat`
        // calls, and only for a query that asked for package work.
        Ok(package_answers(
            packages::detect().as_ref(),
            capabilities,
            text,
            query,
        ))
    }
}

/// Build the answers for one package query.
///
/// Every path here produces a result. A missing package manager, a missing
/// tool, an absent authorisation agent and an impossible package name each
/// become a result that says so, because a user who asked for the capability
/// is owed an answer rather than an empty list.
fn package_answers(
    detected: Option<&Detected>,
    capabilities: &[Capability],
    text: &str,
    query: &str,
) -> Vec<Item> {
    let Some(detected) = detected else {
        return vec![answer_message(
            "packages.unavailable",
            "Packages Unavailable",
            &Unsupported::NoAdapter.message(),
            query,
        )];
    };

    let term = match Term::parse(text) {
        Ok(term) => term,
        Err(error) => {
            return vec![answer_message(
                "packages.term",
                "Package Name Needed",
                &Unsupported::Term(error).message(),
                query,
            )];
        }
    };

    capabilities
        .iter()
        .map(|capability| match detected.plan(*capability, Some(&term)) {
            Ok(plan) => package_answer(&plan, &term, query),
            Err(unsupported) => answer_message(
                &format!("packages.{}", identifier(*capability)),
                capability_title(*capability, &term),
                &unsupported.message(),
                query,
            ),
        })
        .collect()
}

fn package_status_term(query: &str) -> Option<String> {
    let query = query.trim();
    ["pkg ", "package "]
        .into_iter()
        .find_map(|prefix| query.strip_prefix(prefix))
        .map(str::trim)
        .filter(|term| !term.is_empty())
        .map(str::to_string)
}

struct PackageSnapshot {
    term: Term,
    family: packages::Family,
    search: String,
    metadata: String,
    installed: bool,
    plans: Vec<packages::Plan>,
}

fn package_snapshot(term: &str) -> Result<PackageSnapshot, String> {
    let term = Term::parse(term).map_err(|error| error.message())?;
    let detected = packages::detect().ok_or_else(|| Unsupported::NoAdapter.message())?;
    let family = detected.family();
    let cancellation = crate::system::CancellationToken::new();
    let mut plans = Vec::new();
    let mut search = String::new();
    let mut metadata = String::new();
    let mut installed = false;
    for capability in [
        Capability::Search,
        Capability::Metadata,
        Capability::Installed,
    ] {
        let plan = detected
            .plan(capability, Some(&term))
            .map_err(|error| error.message())?;
        let output = crate::system::run(&plan.spec, &cancellation)
            .map_err(|error| format!("{}: {error:?}", plan.display))?;
        let text = if output.stdout.trim().is_empty() {
            output.stderr.trim().to_string()
        } else {
            output.stdout.trim().to_string()
        };
        match capability {
            Capability::Search => search = text,
            Capability::Metadata => metadata = text,
            Capability::Installed => installed = !output.stdout.trim().is_empty(),
            _ => unreachable!(),
        }
        plans.push(plan);
    }
    for capability in [Capability::Install, Capability::Remove] {
        if let Ok(plan) = detected.plan(capability, Some(&term)) {
            plans.push(plan);
        }
    }
    Ok(PackageSnapshot {
        term,
        family,
        search,
        metadata,
        installed,
        plans,
    })
}

fn merged_package_item(snapshot: PackageSnapshot, query: &str) -> Item {
    let term = snapshot.term.as_str();
    let status = if snapshot.installed {
        "Installed"
    } else if snapshot.search.is_empty() {
        "Not found in configured repositories"
    } else {
        "Available to install"
    };
    let detail = if snapshot.metadata.is_empty() {
        snapshot.search.clone()
    } else {
        snapshot.metadata.clone()
    };
    let mut item = answer_item(
        "packages.merged",
        term,
        &format!("{status} · {}", snapshot.family.label()),
        Kind::Package,
        "package-x-generic",
        query,
        Action::Message {
            text: if detail.is_empty() {
                format!("{term}: {status}")
            } else {
                detail.clone()
            },
        },
    );
    item.category = Some(status.into());
    for plan in &snapshot.plans {
        let candidate = package_answer(plan, &snapshot.term, query);
        let include = match plan.capability {
            Capability::Install => !snapshot.installed,
            Capability::Remove => snapshot.installed,
            _ => true,
        };
        if include {
            item.secondary_actions.push(crate::search::ItemAction {
                id: candidate.id,
                label: capability_title(plan.capability, &snapshot.term),
                action: candidate.action,
            });
        }
    }
    item
}

fn detected_adapter() -> Result<Detected, IntegrationError> {
    packages::detect().ok_or_else(|| IntegrationError {
        message: Unsupported::NoAdapter.message(),
    })
}

/// Split a query into the capabilities its first word asks for and the text
/// that follows. Provider-local and deliberately small; the general parsing
/// layer is later product work.
fn package_keyword(query: &str) -> Option<(&'static [Capability], &str)> {
    let query = query.trim();
    let (keyword, rest) = query.split_once(char::is_whitespace).unwrap_or((query, ""));
    let keyword = keyword.to_lowercase();
    PACKAGE_KEYWORDS
        .iter()
        .find(|(candidate, _)| *candidate == keyword)
        .map(|(_, capabilities)| (*capabilities, rest))
}

fn identifier(capability: Capability) -> &'static str {
    match capability {
        Capability::Search => "search",
        Capability::Metadata => "metadata",
        Capability::Installed => "installed",
        Capability::Updates => "updates",
        Capability::Install => "install",
        Capability::Remove => "remove",
    }
}

fn capability_title(capability: Capability, term: &Term) -> String {
    let term = term.as_str();
    match capability {
        Capability::Search => format!("Search Packages for “{term}”"),
        Capability::Metadata => format!("Package Details for “{term}”"),
        Capability::Installed => format!("Is “{term}” Installed?"),
        Capability::Updates => "Check for Package Updates".into(),
        Capability::Install => format!("Install “{term}”"),
        Capability::Remove => format!("Remove “{term}”"),
    }
}

/// The result row for one planned package operation. Its subtitle is the exact
/// command, unexpanded, so the user can read what Enter will run.
fn package_answer(plan: &packages::Plan, term: &Term, query: &str) -> Item {
    let id = format!("packages.{}", identifier(plan.capability));
    let title = capability_title(plan.capability, term);
    let action = if plan.capability.mutates() {
        ProcessAction::mutating(
            id.clone(),
            title.clone(),
            plan.spec.clone(),
            Confirmation {
                summary: format!(
                    "{} the {} package “{}”. This changes the system and needs the desktop's authorisation.",
                    match plan.capability {
                        Capability::Remove => "Removes",
                        _ => "Installs",
                    },
                    plan.family.label(),
                    term.as_str()
                ),
                target: plan.display.clone(),
            },
        )
    } else {
        ProcessAction::read_only(id.clone(), title.clone(), plan.spec.clone())
    };

    Item {
        id,
        provider: String::new(),
        provider_title: String::new(),
        provider_priority: 0,
        title,
        subtitle: plan.display.clone(),
        kind: Kind::Package,
        icon: search::themed(if plan.capability.mutates() {
            "system-software-install"
        } else {
            "package-x-generic"
        }),
        category: Some(plan.family.label().to_string()),
        keywords: answer_keywords(query, term),
        action: Action::Process { action },
        secondary_actions: Vec::new(),
    }
}

/// An answer that has to explain itself instead of running. A capability the
/// session cannot offer still produces a result, in words, rather than
/// silently returning nothing.
fn answer_message(id: &str, title: impl Into<String>, message: &str, query: &str) -> Item {
    Item {
        id: id.into(),
        provider: String::new(),
        provider_title: String::new(),
        provider_priority: 0,
        title: title.into(),
        subtitle: message.into(),
        kind: Kind::Package,
        icon: search::themed("dialog-warning-symbolic"),
        category: Some("Packages".into()),
        keywords: vec![query.trim().to_lowercase()],
        action: Action::Message {
            text: message.into(),
        },
        secondary_actions: Vec::new(),
    }
}

/// An answer was produced *for* this query, so the query itself is one of its
/// keywords. That is what keeps a generated result matchable by the same
/// deterministic ranker as everything else, with no special case in `search`.
fn answer_keywords(query: &str, term: &Term) -> Vec<String> {
    vec![
        query.trim().to_lowercase(),
        term.as_str().to_lowercase(),
        "package".into(),
    ]
}

/// A static item that names the tool it is missing rather than vanishing.
fn unsupported_item(id: &str, title: &str, unsupported: &Unsupported, keywords: &[&str]) -> Item {
    Item {
        id: id.into(),
        provider: String::new(),
        provider_title: String::new(),
        provider_priority: 0,
        title: title.into(),
        subtitle: unsupported.message(),
        kind: Kind::Scene,
        icon: search::themed("dialog-warning-symbolic"),
        category: Some("Packages".into()),
        keywords: keywords.iter().map(|keyword| (*keyword).into()).collect(),
        action: Action::Message {
            text: unsupported.message(),
        },
        secondary_actions: Vec::new(),
    }
}

fn process_item(
    id: &str,
    title: &str,
    subtitle: impl Into<String>,
    category: &str,
    keywords: &[&str],
    action: ProcessAction,
) -> Item {
    Item {
        id: id.into(),
        provider: String::new(),
        provider_title: String::new(),
        provider_priority: 0,
        title: title.into(),
        subtitle: subtitle.into(),
        kind: Kind::Scene,
        icon: search::themed("system-run-symbolic"),
        category: Some(category.into()),
        keywords: keywords.iter().map(|keyword| (*keyword).into()).collect(),
        action: Action::Process { action },
        secondary_actions: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::{self, ExecutionPolicy};

    #[test]
    fn a_provider_error_becomes_one_local_unavailable_result() {
        let item = unavailable_item(
            Metadata {
                id: "broken",
                title: "Broken",
                description: "",
                default_priority: 99,
            },
            IntegrationError {
                message: "offline".into(),
            },
        );
        assert_eq!(item.id, "provider.broken.unavailable");
        assert!(matches!(item.action, Action::Message { .. }));
    }

    #[test]
    fn a_package_keyword_is_recognised_with_its_term() {
        let (capabilities, term) = package_keyword("pkg ripgrep").expect("a package query");
        assert_eq!(
            capabilities,
            &[
                Capability::Search,
                Capability::Metadata,
                Capability::Installed
            ]
        );
        assert_eq!(term, "ripgrep");

        assert_eq!(
            package_keyword("INSTALL gtk4-devel"),
            Some((&[Capability::Install][..], "gtk4-devel"))
        );
        assert_eq!(
            package_keyword("  uninstall vim  ").map(|(c, _)| c),
            Some(&[Capability::Remove][..])
        );
    }

    #[test]
    fn a_keyword_without_a_term_still_reaches_the_provider() {
        // So the answer can say what is missing, rather than nothing at all.
        let (_, term) = package_keyword("install").expect("a package query");
        assert!(term.is_empty());
    }

    #[test]
    fn an_ordinary_query_is_not_package_work() {
        for query in ["firefox", "packages", "installer", "remover", ""] {
            assert!(package_keyword(query).is_none(), "{query}");
        }
    }

    #[test]
    fn a_query_that_names_no_provider_produces_no_answers() {
        // The answer path must stay silent for the queries that make up almost
        // all typing, whatever this machine has installed.
        let config = test_config("/tmp/scene-test");
        assert!(Packages.answer("firefox", &config).unwrap().is_empty());
        assert!(answers("").is_empty());
    }

    #[test]
    fn an_answer_carries_its_own_query_as_a_keyword() {
        // That is what lets the shared ranker find a generated result without
        // `search` needing a special case for it.
        let term = Term::parse("ripgrep").unwrap();
        let keywords = answer_keywords("  PKG ripgrep ", &term);
        assert_eq!(keywords[0], "pkg ripgrep");
        assert!(keywords.contains(&"ripgrep".to_string()));
    }

    /// A machine that has exactly these executables and nothing else.
    fn machine(programs: &'static [&'static str]) -> Option<Detected> {
        crate::packages::detect_with(
            &[],
            Box::new(move |program: &str| {
                programs
                    .contains(&program)
                    .then(|| PathBuf::from(format!("/usr/bin/{program}")))
            }),
        )
    }

    fn answered(machine: Option<Detected>, query: &str) -> Vec<Item> {
        let (capabilities, text) = package_keyword(query).expect("a package query");
        package_answers(machine.as_ref(), capabilities, text, query)
    }

    #[test]
    fn a_package_query_answers_with_the_exact_command_it_would_run() {
        let items = answered(machine(&["dnf", "rpm"]), "pkg ripgrep");
        let titles: Vec<&str> = items.iter().map(|item| item.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Search Packages for “ripgrep”",
                "Package Details for “ripgrep”",
                "Is “ripgrep” Installed?"
            ]
        );
        assert_eq!(
            items[0].subtitle, "dnf --quiet --assumeno search ripgrep",
            "the row must show what Enter will run"
        );
        assert!(items.iter().all(|item| item.kind == Kind::Package));
    }

    #[test]
    fn an_install_answer_is_mutating_and_names_its_target() {
        let items = answered(machine(&["pacman", "pkexec"]), "install ripgrep");
        let [item] = &items[..] else {
            panic!("one install answer, got {}", items.len());
        };
        assert!(actions::requires_confirmation(&item.action));

        let Action::Process { action } = &item.action else {
            panic!("an install must be a registered process");
        };
        assert_eq!(action.policy, ExecutionPolicy::Mutating);
        let text = actions::confirmation_text(action);
        assert!(text.contains("ripgrep"), "{text}");
        assert!(text.contains("pkexec pacman"), "{text}");
        assert!(text.contains("Escape to cancel"), "{text}");
    }

    #[test]
    fn a_read_only_package_query_is_never_mutating() {
        for item in answered(machine(&["apt-cache", "dpkg-query"]), "pkg ripgrep") {
            let Action::Process { action } = &item.action else {
                panic!("{} should be a registered process", item.title);
            };
            assert_eq!(action.policy, ExecutionPolicy::ReadOnly, "{}", item.title);
            assert!(!actions::requires_confirmation(&item.action));
        }
    }

    #[test]
    fn every_unanswerable_package_query_still_produces_a_result() {
        // No package manager at all.
        let none = answered(None, "pkg ripgrep");
        assert_eq!(none.len(), 1);
        assert!(
            none[0].subtitle.contains("apt-cache"),
            "{}",
            none[0].subtitle
        );

        // A package name that cannot be one.
        let refused = answered(machine(&["dnf", "rpm"]), "install --assumeyes");
        assert_eq!(refused.len(), 1);
        assert!(matches!(refused[0].action, Action::Message { .. }));
        assert!(
            refused[0].subtitle.contains("option"),
            "{}",
            refused[0].subtitle
        );

        // A capability whose tool is missing, alongside two that work.
        let partial = answered(machine(&["dnf"]), "pkg ripgrep");
        assert_eq!(partial.len(), 3);
        assert!(matches!(partial[2].action, Action::Message { .. }));
        assert!(
            partial[2].subtitle.contains("rpm"),
            "{}",
            partial[2].subtitle
        );

        // Mutation with no authorisation agent explains itself rather than
        // quietly reaching for sudo.
        let unelevated = answered(machine(&["dnf", "sudo"]), "install ripgrep");
        assert_eq!(unelevated.len(), 1);
        assert!(matches!(unelevated[0].action, Action::Message { .. }));
        assert!(
            unelevated[0].subtitle.contains("sudo"),
            "{}",
            unelevated[0].subtitle
        );

        // Every one of them is findable by the query that produced it.
        for items in [none, refused, partial, unelevated] {
            for item in items {
                assert!(
                    !crate::search::search(
                        &item.keywords[0].clone(),
                        &[&item],
                        &crate::search::History::disabled()
                    )
                    .is_empty(),
                    "{} is unreachable from its own query",
                    item.title
                );
            }
        }
    }

    #[test]
    fn every_capability_has_a_stable_identifier_and_a_title() {
        let term = Term::parse("ripgrep").unwrap();
        let mut seen = Vec::new();
        for capability in Capability::ALL {
            let id = identifier(capability);
            assert!(!seen.contains(&id), "duplicate identifier {id}");
            seen.push(id);
            assert!(capability_title(capability, &term).len() > 3);
        }
    }

    #[test]
    fn directory_configuration_uses_the_explicit_value() {
        let config = test_config("/tmp/scene-test");
        let item = ConfiguredDirectory.search(&config).unwrap().pop().unwrap();
        assert_eq!(item.subtitle, "/tmp/scene-test");
    }

    #[test]
    fn calculator_answers_are_copyable() {
        let [item] = &Calculator.answer("= 2 + 2", &test_config("/tmp")).unwrap()[..] else {
            panic!("one calculator answer expected");
        };
        assert_eq!(item.title, "4");
        assert!(matches!(&item.action, Action::Copy { text, .. } if text == "4"));
    }

    #[test]
    fn named_timezones_convert_deterministically() {
        let query = "time 2026-08-24 14:30 America/Chicago to Europe/London";
        let [item] = &DateAndTime.answer(query, &test_config("/tmp")).unwrap()[..] else {
            panic!("one timezone answer expected");
        };
        assert_eq!(item.title, "2026-08-24 8:30 PM BST");
    }

    #[test]
    fn colours_and_unicode_expose_copy_actions() {
        let [color] = &Colors
            .answer("color #ff0000", &test_config("/tmp"))
            .unwrap()[..]
        else {
            panic!("one colour answer expected");
        };
        assert_eq!(color.secondary_actions.len(), 2);
        assert!(matches!(color.action, Action::Copy { .. }));

        let [character] = &Characters
            .answer("char U+1F680", &test_config("/tmp"))
            .unwrap()[..]
        else {
            panic!("one character answer expected");
        };
        assert_eq!(character.title, "🚀");
        assert!(character.subtitle.contains("ROCKET"));
    }

    #[test]
    fn currency_syntax_and_ecb_rates_are_parsed_without_network_access() {
        let request = CurrencyRequest::parse("convert 10 usd to gbp").expect("currency query");
        assert_eq!(request.amount, 10.0);
        assert_eq!(request.source, "USD");
        assert_eq!(request.target, "GBP");
        assert!(CurrencyRequest::parse("convert ten USD to GBP").is_none());

        let rates = parse_currency_rates(
            r#"<Envelope><Cube><Cube time="2026-08-24"><Cube currency="USD" rate="1.2"/><Cube currency="GBP" rate="0.8"/></Cube></Cube></Envelope>"#,
        )
        .expect("valid ECB fixture");
        assert_eq!(rates["EUR"], 1.0);
        assert_eq!(rates["USD"], 1.2);
        assert_eq!(rates["GBP"], 0.8);
    }

    #[test]
    fn xbel_entries_allow_bookmarks_without_titles() {
        let path = std::env::temp_dir().join(format!(
            "scene-xbel-{}-{}.xbel",
            std::process::id(),
            stable_id("missing-title")
        ));
        std::fs::write(
            &path,
            r#"<xbel><bookmark href="file:///tmp/untitled"/><bookmark href="file:///tmp/named"><title>Named place</title></bookmark></xbel>"#,
        )
        .unwrap();
        let entries = xbel_entries(&path).unwrap();
        let _ = std::fs::remove_file(path);
        assert_eq!(
            entries,
            [
                ("file:///tmp/untitled".into(), None),
                ("file:///tmp/named".into(), Some("Named place".into()))
            ]
        );
    }

    #[test]
    fn explicit_commands_are_argv_based_bounded_and_confirmed() {
        let Some(executable) = system::locate("printf") else {
            return;
        };
        let config = test_config("/tmp/scene-test");
        let item = command_item("printf '%s' 'two words'", &config, 0).unwrap();
        let Action::Process { action } = &item.action else {
            panic!("a command is a typed process action");
        };
        assert_eq!(action.spec.program, executable.to_string_lossy());
        assert_eq!(action.spec.args, ["%s", "two words"]);
        assert_eq!(
            action.spec.working_directory.as_deref(),
            Some("/tmp/scene-test")
        );
        assert_eq!(action.spec.timeout, Duration::from_secs(30));
        assert_eq!(action.spec.output_limit, 64 * 1024);
        assert!(actions::requires_confirmation(&item.action));
    }

    #[test]
    fn provider_order_and_enablement_are_explicit_configuration() {
        let mut config = test_config("/tmp");
        for (priority, id) in ["one", "two", "three"].into_iter().enumerate() {
            config.providers.insert(
                id.into(),
                ProviderPreference {
                    enabled: true,
                    priority: priority as u16,
                },
            );
        }
        config.set_provider_enabled("two", false);
        config.move_provider("three", -1);
        assert!(!config.provider_enabled("two"));
        assert_eq!(config.ordered_provider_ids(), ["one", "three", "two"]);
    }

    #[test]
    fn persistent_configuration_is_versioned_and_complete() {
        let mut config = test_config("/tmp/scene configured");
        config.history_enabled = false;
        config.command_history_enabled = true;
        config.file_content_enabled = true;
        config.providers.insert(
            "files".into(),
            ProviderPreference {
                enabled: false,
                priority: 7,
            },
        );
        let file = config.key_file();
        assert_eq!(file.integer("format", "version"), Ok(1));
        assert_eq!(
            file.string("general", "directory").unwrap(),
            "/tmp/scene configured"
        );
        assert_eq!(file.boolean("general", "history-enabled"), Ok(false));
        assert_eq!(file.boolean("general", "command-history-enabled"), Ok(true));
        assert_eq!(file.boolean("general", "file-content-enabled"), Ok(true));
        assert_eq!(file.boolean("provider files", "enabled"), Ok(false));
        assert_eq!(file.integer("provider files", "priority"), Ok(7));
    }

    fn test_config(path: &str) -> Config {
        Config {
            directory: DirectoryConfig {
                path: PathBuf::from(path),
            },
            providers: BTreeMap::new(),
            history_enabled: true,
            command_history_enabled: false,
            file_content_enabled: false,
        }
    }
}
