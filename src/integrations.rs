//! Built-in integration contracts and registry.
//!
//! Providers may return an error, but the registry converts it to one local
//! unavailable result. A faulty provider can therefore never remove another
//! provider's results or break the launcher surface.

use std::path::PathBuf;

use crate::actions::{Action, Confirmation, ProcessAction};
use crate::packages::{self, Capability, Detected, Term, Unsupported};
use crate::search::{self, Item, Kind};
use crate::system::{self, CommandSpec};

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

const PROVIDERS: [&dyn Integration; 5] = [
    &Applications,
    &Terminal,
    &SystemInformation,
    &ConfiguredDirectory,
    &Packages,
];

/// Discover every built-in provider. Errors remain visible and local.
pub fn index() -> Vec<Item> {
    let config = Config::load();
    collect(|provider| provider.search(&config))
}

/// The providers' answers to one query, ranked alongside the static index.
pub fn answers(query: &str) -> Vec<Item> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let config = Config::load();
    collect(|provider| provider.answer(query, &config))
}

/// One provider's error becomes one local result, never a missing group.
fn collect(
    mut ask: impl FnMut(&dyn Integration) -> Result<Vec<Item>, IntegrationError>,
) -> Vec<Item> {
    PROVIDERS
        .into_iter()
        .flat_map(|provider| {
            let metadata = provider.metadata();
            match ask(provider) {
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
    }
}

/// An answer that has to explain itself instead of running. A capability the
/// session cannot offer still produces a result, in words, rather than
/// silently returning nothing.
fn answer_message(id: &str, title: impl Into<String>, message: &str, query: &str) -> Item {
    Item {
        id: id.into(),
        title: title.into(),
        subtitle: message.into(),
        kind: Kind::Package,
        icon: search::themed("dialog-warning-symbolic"),
        category: Some("Packages".into()),
        keywords: vec![query.trim().to_lowercase()],
        action: Action::Message {
            text: message.into(),
        },
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
        title: title.into(),
        subtitle: unsupported.message(),
        kind: Kind::Scene,
        icon: search::themed("dialog-warning-symbolic"),
        category: Some("Packages".into()),
        keywords: keywords.iter().map(|keyword| (*keyword).into()).collect(),
        action: Action::Message {
            text: unsupported.message(),
        },
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
        let config = Config {
            directory: DirectoryConfig {
                path: PathBuf::from("/tmp/scene-test"),
            },
        };
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
        let config = Config {
            directory: DirectoryConfig {
                path: PathBuf::from("/tmp/scene-test"),
            },
        };
        let item = ConfiguredDirectory.search(&config).unwrap().pop().unwrap();
        assert_eq!(item.subtitle, "/tmp/scene-test");
    }
}
