//! Built-in integration contracts and registry.
//!
//! Providers may return an error, but the registry converts it to one local
//! unavailable result. A faulty provider can therefore never remove another
//! provider's results or break the launcher surface.

use std::path::PathBuf;

use crate::actions::{Action, ProcessAction};
use crate::search::{self, Item, Kind};
use crate::system::CommandSpec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Metadata {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectoryConfig {
    pub path: PathBuf,
}

/// Configuration is explicit and typed. Persistent configuration/migration is
/// a later packaging milestone; `SCENE_DIRECTORY` is the current user-facing
/// override and defaults to the home directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub directory: DirectoryConfig,
}

impl Config {
    pub fn load() -> Self {
        let path = std::env::var_os("SCENE_DIRECTORY")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("/"))
            });
        Self {
            directory: DirectoryConfig { path },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntegrationError {
    pub message: String,
}

/// The search contract: each provider returns stable, renderable items for the
/// central deterministic matcher. It cannot return widgets or commands.
pub trait Integration {
    fn metadata(&self) -> Metadata;
    fn search(&self, config: &Config) -> Result<Vec<Item>, IntegrationError>;
}

/// Discover every built-in provider. Errors remain visible and local.
pub fn index() -> Vec<Item> {
    let config = Config::load();
    let providers: [&dyn Integration; 5] = [
        &Applications,
        &Terminal,
        &SystemInformation,
        &ConfiguredDirectory,
        &PackageManager,
    ];

    providers
        .into_iter()
        .flat_map(|provider| {
            let metadata = provider.metadata();
            match provider.search(&config) {
                Ok(items) => items,
                Err(error) => vec![unavailable_item(metadata, error)],
            }
        })
        .collect()
}

fn unavailable_item(metadata: Metadata, error: IntegrationError) -> Item {
    Item {
        id: format!("provider.{}.unavailable", metadata.id),
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
    }
}

struct Applications;

impl Integration for Applications {
    fn metadata(&self) -> Metadata {
        Metadata {
            id: "applications",
            title: "Applications",
            description: "Installed desktop applications",
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
        }
    }

    fn search(&self, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        let program = ["konsole", "kgx", "gnome-terminal", "foot", "xterm"]
            .into_iter()
            .find(|program| executable_on_path(program))
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
        }
    }

    fn search(&self, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        if !executable_on_path("uname") {
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
        }
    }

    fn search(&self, config: &Config) -> Result<Vec<Item>, IntegrationError> {
        let path = config.directory.path.to_string_lossy().into_owned();
        Ok(vec![Item {
            id: "directory.configured".into(),
            title: "Open Configured Directory".into(),
            subtitle: path.clone(),
            kind: Kind::Folder,
            icon: search::themed("folder"),
            category: Some("Directory".into()),
            keywords: vec!["folder".into(), "files".into(), "scene_directory".into()],
            action: Action::Open { target: path },
        }])
    }
}

struct PackageManager;

impl Integration for PackageManager {
    fn metadata(&self) -> Metadata {
        Metadata {
            id: "package-manager",
            title: "Package manager",
            description: "Read-only detected package-manager details",
        }
    }

    fn search(&self, _: &Config) -> Result<Vec<Item>, IntegrationError> {
        let (program, args): (&str, &[&str]) = [
            ("apt-cache", &["--version"] as &[&str]),
            ("dnf", &["--version"]),
            ("pacman", &["--version"]),
        ]
        .into_iter()
        .find(|(program, _)| executable_on_path(program))
        .ok_or_else(|| IntegrationError {
            message: "No supported package manager is installed (apt-cache, dnf, or pacman)."
                .into(),
        })?;
        Ok(vec![process_item(
            "packages.manager-info",
            "Show Package Manager Information",
            format!("Read-only query through {program}"),
            "Packages",
            &["package", "apt", "dnf", "pacman", "version"],
            ProcessAction::read_only(
                "packages.manager-info",
                "Show Package Manager Information",
                CommandSpec::read_only(program, args.iter().copied()),
            ),
        )])
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
        title: title.into(),
        subtitle: subtitle.into(),
        kind: Kind::Scene,
        icon: search::themed("system-run-symbolic"),
        category: Some(category.into()),
        keywords: keywords.iter().map(|keyword| (*keyword).into()).collect(),
        action: Action::Process { action },
    }
}

fn executable_on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|directory| directory.join(program).is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_provider_error_becomes_one_local_unavailable_result() {
        let item = unavailable_item(
            Metadata {
                id: "broken",
                title: "Broken",
                description: "",
            },
            IntegrationError {
                message: "offline".into(),
            },
        );
        assert_eq!(item.id, "provider.broken.unavailable");
        assert!(matches!(item.action, Action::Message { .. }));
    }

    #[test]
    fn directory_configuration_uses_the_explicit_value() {
        let config = Config {
            directory: DirectoryConfig {
                path: PathBuf::from("/tmp/scene-test"),
            },
        };
        let item = ConfiguredDirectory.search(&config).unwrap().pop().unwrap();
        assert_eq!(item.subtitle, "/tmp/scene-test");
    }
}
