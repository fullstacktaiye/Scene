//! Distro capability adapters.
//!
//! One interface over the Debian, Fedora and Arch package tools. A capability
//! is real when its executable is found on `PATH`. The distribution name in
//! `/etc/os-release` only decides which adapter is *probed first*; it can
//! never make a capability available on its own, and a machine that carries
//! another family's tooling is detected by that tooling.
//!
//! Nothing here runs a process. An adapter answers with a fully specified
//! [`Plan`] — program, argument vector, timeout, output limit and accepted
//! exit codes — which `actions` and `system` then execute under the same
//! bounded policy as every other Scene action.

use std::path::PathBuf;
use std::time::Duration;

use crate::system::{self, CommandSpec};

/// Package work is slower than the three-second default: a search may refresh
/// repository metadata, and an install waits on an authorisation prompt.
const QUERY_TIMEOUT: Duration = Duration::from_secs(20);
const LOCAL_TIMEOUT: Duration = Duration::from_secs(8);
const UPDATES_TIMEOUT: Duration = Duration::from_secs(60);
const MUTATION_TIMEOUT: Duration = Duration::from_secs(900);
const OUTPUT_LIMIT: usize = 32 * 1024;

/// Exit codes that answer a question rather than report a failure. Verified
/// on Debian stable, Fedora 44 and Arch: `dpkg-query`, `rpm --query` and
/// `pacman --query --info` all exit 1 for a package that is simply not
/// installed, and `pacman --sync --search` exits 1 when nothing matches.
/// Asking whether something is there and being told "no" is an answer.
const NOT_INSTALLED: &[i32] = &[1];
const NO_MATCH: &[i32] = &[1];
/// `dnf check-update` exits 100 when there is something to update.
const UPDATES_AVAILABLE: &[i32] = &[100];
/// `pacman --query --upgrades` exits 1 when there is not.
const NOTHING_TO_UPGRADE: &[i32] = &[1];

/// The desktop's own way to ask for authorisation. Scene never assumes `sudo`
/// and never runs its own password prompt; if PolicyKit is absent, mutation is
/// reported unsupported instead.
const ELEVATION: &str = "pkexec";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Debian,
    Fedora,
    Arch,
}

impl Family {
    pub fn label(self) -> &'static str {
        match self {
            Family::Debian => "Debian/Ubuntu",
            Family::Fedora => "Fedora",
            Family::Arch => "Arch",
        }
    }

    /// `ID` and `ID_LIKE` values that hint at this family. A hint only orders
    /// the probe.
    fn hints(self) -> &'static [&'static str] {
        match self {
            Family::Debian => &["debian", "ubuntu", "linuxmint", "pop", "raspbian"],
            Family::Fedora => &["fedora", "rhel", "centos", "almalinux", "rocky"],
            Family::Arch => &["arch", "archlinux", "manjaro", "endeavouros"],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capability {
    Search,
    Metadata,
    Installed,
    Updates,
    Install,
    Remove,
}

impl Capability {
    /// Every capability the adapter interface defines, in a stable order.
    pub const ALL: [Capability; 6] = [
        Capability::Search,
        Capability::Metadata,
        Capability::Installed,
        Capability::Updates,
        Capability::Install,
        Capability::Remove,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Capability::Search => "package search",
            Capability::Metadata => "package metadata",
            Capability::Installed => "installed-package lookup",
            Capability::Updates => "update availability",
            Capability::Install => "package install",
            Capability::Remove => "package removal",
        }
    }

    /// Whether the capability names one package. `Updates` asks about the
    /// system as a whole.
    pub fn takes_term(self) -> bool {
        !matches!(self, Capability::Updates)
    }

    /// Whether the capability changes durable system state, and so must be
    /// confirmed and elevated.
    pub fn mutates(self) -> bool {
        matches!(self, Capability::Install | Capability::Remove)
    }
}

/// A package name that has been checked before it is allowed near an argument
/// vector.
///
/// This is the one place where text a user typed reaches a command, so the
/// rules are deliberately narrow: the characters a package name can actually
/// contain, nothing else, and never a leading dash that a tool would read as
/// an option. There is still no shell involved — the term becomes one element
/// of an `execve` argument vector, not a word in a command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Term(String);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TermError {
    Empty,
    TooLong,
    LeadingDash,
    Character(char),
}

impl TermError {
    pub fn message(self) -> String {
        match self {
            TermError::Empty => "Type a package name after the keyword.".into(),
            TermError::TooLong => format!(
                "A package name is at most {} characters long.",
                Term::MAX_LENGTH
            ),
            TermError::Character(character) => format!(
                "“{character}” cannot appear in a package name. Use letters, digits, and - _ . + only."
            ),
            TermError::LeadingDash => {
                "A package name cannot start with “-”, which a package tool would read as an option."
                    .into()
            }
        }
    }
}

impl Term {
    pub const MAX_LENGTH: usize = 128;

    pub fn parse(text: &str) -> Result<Self, TermError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(TermError::Empty);
        }
        if text.chars().count() > Self::MAX_LENGTH {
            return Err(TermError::TooLong);
        }
        if text.starts_with('-') {
            return Err(TermError::LeadingDash);
        }
        match text.chars().find(|character| !Self::permits(*character)) {
            Some(character) => Err(TermError::Character(character)),
            None => Ok(Self(text.to_string())),
        }
    }

    fn permits(character: char) -> bool {
        character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '+')
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What an adapter declares for one capability, before Scene resolves paths.
struct Recipe {
    /// The executable this capability needs. Its presence on `PATH` is what
    /// makes the capability real.
    program: &'static str,
    args: Vec<String>,
    timeout: Duration,
    accepted_exit_codes: &'static [i32],
    elevated: bool,
}

/// One package operation, fully specified before anything runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub family: Family,
    pub capability: Capability,
    /// The exact command, with every program resolved to an absolute path.
    pub spec: CommandSpec,
    /// The same command in words the user can check before confirming it,
    /// unresolved and unexpanded.
    pub display: String,
    pub elevated: bool,
}

/// Why a capability cannot be offered here. Every arm says so in words,
/// because a user who searched for the capability is owed an answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unsupported {
    NoAdapter,
    Capability {
        family: Family,
        capability: Capability,
    },
    Tool {
        capability: Capability,
        tool: &'static str,
    },
    Elevation {
        capability: Capability,
    },
    Term(TermError),
}

impl Unsupported {
    pub fn message(&self) -> String {
        match self {
            Unsupported::NoAdapter => {
                "No supported package manager is installed. Scene looks for apt-cache, dnf, and pacman on PATH."
                    .into()
            }
            Unsupported::Capability { family, capability } => format!(
                "The {} adapter does not provide {}.",
                family.label(),
                capability.label()
            ),
            Unsupported::Tool { capability, tool } => format!(
                "{tool} is not installed, or not on PATH, so {} is unavailable.",
                capability.label()
            ),
            Unsupported::Elevation { capability } => format!(
                "{ELEVATION} is not installed, so {} cannot ask the desktop for authorisation. Scene will not fall back to sudo.",
                capability.label()
            ),
            Unsupported::Term(error) => error.message(),
        }
    }
}

trait Adapter {
    fn family(&self) -> Family;

    /// The executable whose presence means this family's tooling is the one
    /// installed here.
    fn signature(&self) -> &'static str;

    /// The command for a capability. `term` is `None` only while Scene is
    /// checking whether the capability exists at all; the arguments returned
    /// then are incomplete and are never executed, because [`Detected::plan`]
    /// refuses a term-taking capability without a term.
    fn command(&self, capability: Capability, term: Option<&Term>) -> Option<Recipe>;
}

/// Append the validated term, when there is one, to a fixed argument vector.
/// Every supported command takes the package name last.
fn vector(fixed: &[&str], term: Option<&Term>) -> Vec<String> {
    let mut args: Vec<String> = fixed
        .iter()
        .map(|argument| (*argument).to_string())
        .collect();
    if let Some(term) = term {
        args.push(term.as_str().to_string());
    }
    args
}

fn recipe(program: &'static str, args: Vec<String>, timeout: Duration) -> Recipe {
    Recipe {
        program,
        args,
        timeout,
        accepted_exit_codes: &[],
        elevated: false,
    }
}

impl Recipe {
    fn accepting(mut self, codes: &'static [i32]) -> Self {
        self.accepted_exit_codes = codes;
        self
    }

    fn elevated(mut self) -> Self {
        self.elevated = true;
        self
    }
}

struct Debian;

impl Adapter for Debian {
    fn family(&self) -> Family {
        Family::Debian
    }

    fn signature(&self) -> &'static str {
        "apt-cache"
    }

    fn command(&self, capability: Capability, term: Option<&Term>) -> Option<Recipe> {
        Some(match capability {
            Capability::Search => recipe(
                "apt-cache",
                vector(&["search", "--names-only"], term),
                QUERY_TIMEOUT,
            ),
            Capability::Metadata => recipe("apt-cache", vector(&["show"], term), QUERY_TIMEOUT),
            Capability::Installed => recipe(
                "dpkg-query",
                vector(
                    &[
                        "--show",
                        "--showformat=${binary:Package} ${Version} ${db:Status-Status}\\n",
                    ],
                    term,
                ),
                LOCAL_TIMEOUT,
            )
            .accepting(NOT_INSTALLED),
            // `apt list` warns on stderr that it has no stable CLI interface.
            // Scene shows that warning rather than hiding it.
            Capability::Updates => recipe(
                "apt",
                vector(&["list", "--upgradable"], None),
                UPDATES_TIMEOUT,
            ),
            Capability::Install => recipe(
                "apt-get",
                vector(&["install", "--yes"], term),
                MUTATION_TIMEOUT,
            )
            .elevated(),
            Capability::Remove => recipe(
                "apt-get",
                vector(&["remove", "--yes"], term),
                MUTATION_TIMEOUT,
            )
            .elevated(),
        })
    }
}

struct Fedora;

impl Adapter for Fedora {
    fn family(&self) -> Family {
        Family::Fedora
    }

    fn signature(&self) -> &'static str {
        "dnf"
    }

    fn command(&self, capability: Capability, term: Option<&Term>) -> Option<Recipe> {
        // `--assumeno` answers any prompt dnf raises — a repository key import,
        // for instance — with "no", so a read-only query stays read-only even
        // though its input is closed.
        Some(match capability {
            Capability::Search => recipe(
                "dnf",
                vector(&["--quiet", "--assumeno", "search"], term),
                QUERY_TIMEOUT,
            ),
            Capability::Metadata => recipe(
                "dnf",
                vector(&["--quiet", "--assumeno", "info"], term),
                QUERY_TIMEOUT,
            ),
            // `rpm --query` reads the local database only: no network, and the
            // same answer on dnf4 and dnf5, whose `list` flags differ.
            Capability::Installed => {
                recipe("rpm", vector(&["--query"], term), LOCAL_TIMEOUT).accepting(NOT_INSTALLED)
            }
            // 100 means "updates are available", which is the answer, not a
            // failure.
            Capability::Updates => recipe(
                "dnf",
                vector(&["--quiet", "--assumeno", "check-update"], None),
                UPDATES_TIMEOUT,
            )
            .accepting(UPDATES_AVAILABLE),
            Capability::Install => recipe(
                "dnf",
                vector(&["install", "--assumeyes"], term),
                MUTATION_TIMEOUT,
            )
            .elevated(),
            Capability::Remove => recipe(
                "dnf",
                vector(&["remove", "--assumeyes"], term),
                MUTATION_TIMEOUT,
            )
            .elevated(),
        })
    }
}

struct Arch;

impl Adapter for Arch {
    fn family(&self) -> Family {
        Family::Arch
    }

    fn signature(&self) -> &'static str {
        "pacman"
    }

    fn command(&self, capability: Capability, term: Option<&Term>) -> Option<Recipe> {
        Some(match capability {
            Capability::Search => recipe(
                "pacman",
                vector(&["--sync", "--search"], term),
                QUERY_TIMEOUT,
            )
            .accepting(NO_MATCH),
            Capability::Metadata => {
                recipe("pacman", vector(&["--sync", "--info"], term), QUERY_TIMEOUT)
            }
            Capability::Installed => recipe(
                "pacman",
                vector(&["--query", "--info"], term),
                LOCAL_TIMEOUT,
            )
            .accepting(NOT_INSTALLED),
            // `--query --upgrades` exits 1 when nothing is out of date, which
            // is an answer rather than an error.
            Capability::Updates => recipe(
                "pacman",
                vector(&["--query", "--upgrades"], None),
                UPDATES_TIMEOUT,
            )
            .accepting(NOTHING_TO_UPGRADE),
            Capability::Install => recipe(
                "pacman",
                vector(&["--sync", "--noconfirm"], term),
                MUTATION_TIMEOUT,
            )
            .elevated(),
            Capability::Remove => recipe(
                "pacman",
                vector(&["--remove", "--recursive", "--noconfirm"], term),
                MUTATION_TIMEOUT,
            )
            .elevated(),
        })
    }
}

const ADAPTERS: [&'static dyn Adapter; 3] = [&Debian, &Fedora, &Arch];

/// How Scene finds an executable. Named so a test can describe a machine
/// instead of having to be one.
pub(crate) type Locate = Box<dyn Fn(&str) -> Option<PathBuf>>;

/// The adapter for this machine, or `None` when no supported tooling exists.
pub struct Detected {
    adapter: &'static dyn Adapter,
    locate: Locate,
}

pub fn detect() -> Option<Detected> {
    detect_with(&hinted_families(), Box::new(system::locate))
}

/// The detection rule, with both its inputs made explicit. `integrations`
/// uses the same seam to test its package answers against a stated machine.
pub(crate) fn detect_with(hints: &[Family], locate: Locate) -> Option<Detected> {
    let mut ordered = ADAPTERS;
    // A stable sort, so an unhinted machine keeps the declared order.
    ordered.sort_by_key(|adapter| {
        hints
            .iter()
            .position(|hint| *hint == adapter.family())
            .unwrap_or(usize::MAX)
    });
    let adapter = ordered
        .into_iter()
        .find(|adapter| locate(adapter.signature()).is_some())?;
    Some(Detected { adapter, locate })
}

impl Detected {
    pub fn family(&self) -> Family {
        self.adapter.family()
    }

    /// The capabilities that are usable on this machine right now.
    pub fn capabilities(&self) -> Vec<Capability> {
        Capability::ALL
            .into_iter()
            .filter(|capability| self.available(*capability).is_ok())
            .collect()
    }

    /// Whether a capability could run, without needing a package name for it.
    pub fn available(&self, capability: Capability) -> Result<(), Unsupported> {
        let recipe = self.recipe(capability, None)?;
        self.resolve(capability, &recipe).map(|_| ())
    }

    /// The command Scene would run, or the reason it cannot.
    pub fn plan(&self, capability: Capability, term: Option<&Term>) -> Result<Plan, Unsupported> {
        if capability.takes_term() && term.is_none() {
            return Err(Unsupported::Term(TermError::Empty));
        }
        let recipe = self.recipe(capability, term)?;
        let (program, args) = self.resolve(capability, &recipe)?;

        let mut words = Vec::new();
        if recipe.elevated {
            words.push(ELEVATION.to_string());
        }
        words.push(recipe.program.to_string());
        words.extend(recipe.args.iter().cloned());
        let display = words.join(" ");

        Ok(Plan {
            family: self.family(),
            capability,
            spec: CommandSpec::read_only(program, args)
                .with_timeout(recipe.timeout)
                .with_output_limit(OUTPUT_LIMIT)
                .accepting(recipe.accepted_exit_codes.iter().copied()),
            display,
            elevated: recipe.elevated,
        })
    }

    fn recipe(&self, capability: Capability, term: Option<&Term>) -> Result<Recipe, Unsupported> {
        self.adapter
            .command(capability, term)
            .ok_or(Unsupported::Capability {
                family: self.family(),
                capability,
            })
    }

    /// Turn declared program names into absolute paths, so what runs is not
    /// left to whatever `PATH` happens to say later, and so an elevated
    /// command names its tool unambiguously.
    fn resolve(
        &self,
        capability: Capability,
        recipe: &Recipe,
    ) -> Result<(String, Vec<String>), Unsupported> {
        let tool = (self.locate)(recipe.program).ok_or(Unsupported::Tool {
            capability,
            tool: recipe.program,
        })?;
        let tool = tool.to_string_lossy().into_owned();

        if !recipe.elevated {
            return Ok((tool, recipe.args.clone()));
        }

        let elevation = (self.locate)(ELEVATION).ok_or(Unsupported::Elevation { capability })?;
        let mut args = vec![tool];
        args.extend(recipe.args.iter().cloned());
        Ok((elevation.to_string_lossy().into_owned(), args))
    }
}

/// The families `/etc/os-release` hints at, best first. Never authoritative.
fn hinted_families() -> Vec<Family> {
    let text = std::fs::read_to_string("/etc/os-release")
        .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
        .unwrap_or_default();
    hinted_families_from(&text)
}

fn hinted_families_from(os_release: &str) -> Vec<Family> {
    let mut ids: Vec<String> = Vec::new();
    for line in os_release.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if !matches!(key.trim(), "ID" | "ID_LIKE") {
            continue;
        }
        let value = value.trim().trim_matches('"').trim_matches('\'');
        ids.extend(value.split_whitespace().map(str::to_lowercase));
    }

    let mut families = Vec::new();
    for id in ids {
        for family in [Family::Debian, Family::Fedora, Family::Arch] {
            if family.hints().contains(&id.as_str()) && !families.contains(&family) {
                families.push(family);
            }
        }
    }
    families
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A machine that has exactly these executables and nothing else.
    fn machine(programs: &'static [&'static str]) -> Locate {
        Box::new(move |program: &str| {
            programs
                .contains(&program)
                .then(|| PathBuf::from(format!("/usr/bin/{program}")))
        })
    }

    #[test]
    fn a_package_name_is_accepted_only_in_the_characters_one_can_contain() {
        assert_eq!(Term::parse("ripgrep").unwrap().as_str(), "ripgrep");
        assert_eq!(Term::parse(" gtk4-devel ").unwrap().as_str(), "gtk4-devel");
        assert_eq!(Term::parse("g++").unwrap().as_str(), "g++");
        assert_eq!(Term::parse("python3.13").unwrap().as_str(), "python3.13");
    }

    #[test]
    fn a_term_that_could_be_read_as_an_option_is_refused() {
        assert_eq!(Term::parse("--assumeyes"), Err(TermError::LeadingDash));
        assert_eq!(Term::parse("-y"), Err(TermError::LeadingDash));
    }

    #[test]
    fn shell_punctuation_never_reaches_an_argument_vector() {
        for text in [
            "vim; rm -rf /",
            "vim && reboot",
            "$(reboot)",
            "`reboot`",
            "vim|less",
            "two words",
            "vim\nreboot",
            "../../etc/passwd",
        ] {
            assert!(
                matches!(
                    Term::parse(text),
                    Err(TermError::Character(_)) | Err(TermError::LeadingDash)
                ),
                "{text} should not parse as a package name"
            );
        }
    }

    #[test]
    fn an_empty_or_oversized_term_is_refused() {
        assert_eq!(Term::parse("   "), Err(TermError::Empty));
        assert_eq!(
            Term::parse(&"a".repeat(Term::MAX_LENGTH + 1)),
            Err(TermError::TooLong)
        );
        assert!(Term::parse(&"a".repeat(Term::MAX_LENGTH)).is_ok());
    }

    #[test]
    fn the_installed_executable_decides_the_family_not_the_distribution_name() {
        // A machine that calls itself Fedora but only carries pacman is an
        // Arch machine as far as capability detection is concerned.
        let detected = detect_with(&[Family::Fedora], machine(&["pacman"])).expect("an adapter");
        assert_eq!(detected.family(), Family::Arch);

        assert!(detect_with(&[Family::Fedora], machine(&[])).is_none());
    }

    #[test]
    fn the_os_release_hint_only_orders_the_probe() {
        // Both signatures present: the hint breaks the tie.
        let both: &'static [&'static str] = &["apt-cache", "pacman"];
        assert_eq!(
            detect_with(&[Family::Arch], machine(both))
                .expect("an adapter")
                .family(),
            Family::Arch
        );
        assert_eq!(
            detect_with(&[Family::Debian], machine(both))
                .expect("an adapter")
                .family(),
            Family::Debian
        );
        // No hint: the declared order decides, deterministically.
        assert_eq!(
            detect_with(&[], machine(both))
                .expect("an adapter")
                .family(),
            Family::Debian
        );
    }

    #[test]
    fn os_release_is_read_for_both_id_and_id_like() {
        assert_eq!(
            hinted_families_from("ID=fedora\nVERSION_ID=44\n"),
            vec![Family::Fedora]
        );
        assert_eq!(
            hinted_families_from("ID=ubuntu\nID_LIKE=debian\n"),
            vec![Family::Debian]
        );
        assert_eq!(
            hinted_families_from("ID=\"endeavouros\"\nID_LIKE=\"arch\"\n"),
            vec![Family::Arch]
        );
        assert!(hinted_families_from("ID=plan9\n").is_empty());
        assert!(hinted_families_from("").is_empty());
    }

    #[test]
    fn a_missing_tool_names_itself_rather_than_disappearing() {
        // dnf without rpm: searching works, the installed-package lookup does
        // not, and says which executable is missing.
        let detected = detect_with(&[], machine(&["dnf"])).expect("an adapter");
        assert_eq!(detected.family(), Family::Fedora);

        let error = detected
            .plan(
                Capability::Installed,
                Some(&Term::parse("ripgrep").unwrap()),
            )
            .expect_err("rpm is not installed on this fake machine");
        assert_eq!(
            error,
            Unsupported::Tool {
                capability: Capability::Installed,
                tool: "rpm",
            }
        );
        assert!(error.message().contains("rpm"));
    }

    #[test]
    fn mutation_needs_the_desktop_authorisation_agent_and_never_sudo() {
        let detected = detect_with(&[], machine(&["dnf", "sudo"])).expect("an adapter");
        let term = Term::parse("ripgrep").unwrap();
        let error = detected
            .plan(Capability::Install, Some(&term))
            .expect_err("pkexec is not installed on this fake machine");
        assert_eq!(
            error,
            Unsupported::Elevation {
                capability: Capability::Install
            }
        );
        assert!(error.message().contains("sudo"), "{}", error.message());
    }

    #[test]
    fn a_term_taking_capability_refuses_to_plan_without_a_term() {
        let detected = detect_with(&[], machine(&["dnf", "rpm"])).expect("an adapter");
        assert_eq!(
            detected.plan(Capability::Search, None),
            Err(Unsupported::Term(TermError::Empty))
        );
        // The system-wide capability needs no package name.
        assert!(detected.plan(Capability::Updates, None).is_ok());
    }

    #[test]
    fn the_term_is_one_argument_and_never_part_of_the_program() {
        let detected = detect_with(&[], machine(&["pacman"])).expect("an adapter");
        let term = Term::parse("gtk4").unwrap();
        let plan = detected.plan(Capability::Search, Some(&term)).unwrap();
        assert!(
            plan.spec.program.ends_with("pacman"),
            "{}",
            plan.spec.program
        );
        assert_eq!(plan.spec.args, vec!["--sync", "--search", "gtk4"]);
        assert_eq!(plan.display, "pacman --sync --search gtk4");
        assert!(!plan.elevated);
    }

    #[test]
    fn an_elevated_plan_runs_the_tool_through_the_authorisation_agent() {
        let detected = detect_with(&[], machine(&["dnf", "pkexec"])).expect("an adapter");
        let term = Term::parse("ripgrep").unwrap();
        let plan = detected.plan(Capability::Install, Some(&term)).unwrap();
        assert!(plan.elevated);
        assert!(
            plan.spec.program.ends_with("pkexec"),
            "{}",
            plan.spec.program
        );
        assert!(plan.spec.args[0].ends_with("dnf"), "{}", plan.spec.args[0]);
        assert_eq!(&plan.spec.args[1..], ["install", "--assumeyes", "ripgrep"]);
        assert_eq!(plan.display, "pkexec dnf install --assumeyes ripgrep");
        assert_eq!(plan.spec.timeout, MUTATION_TIMEOUT);
    }

    #[test]
    fn an_answering_exit_code_is_not_a_failure() {
        let fedora = detect_with(&[], machine(&["dnf", "rpm"])).expect("an adapter");
        assert!(
            fedora
                .plan(Capability::Updates, None)
                .unwrap()
                .spec
                .accepted_exit_codes
                .contains(&100)
        );

        let arch = detect_with(&[], machine(&["pacman"])).expect("an adapter");
        assert!(
            arch.plan(Capability::Updates, None)
                .unwrap()
                .spec
                .accepted_exit_codes
                .contains(&1)
        );

        // "That package is not installed" is the answer to the question the
        // result asks, so it must not be reported as a failed command.
        let term = Term::parse("ripgrep").unwrap();
        for (programs, family) in [
            (&["apt-cache", "dpkg-query"][..], Family::Debian),
            (&["dnf", "rpm"][..], Family::Fedora),
            (&["pacman"][..], Family::Arch),
        ] {
            let detected = detect_with(&[], machine(programs)).expect("an adapter");
            assert_eq!(detected.family(), family);
            assert!(
                detected
                    .plan(Capability::Installed, Some(&term))
                    .unwrap()
                    .spec
                    .accepted_exit_codes
                    .contains(&1),
                "{family:?} must treat “not installed” as an answer"
            );
        }
    }

    #[test]
    fn every_family_declares_every_capability() {
        let term = Term::parse("ripgrep").unwrap();
        for adapter in ADAPTERS {
            for capability in Capability::ALL {
                let recipe = adapter
                    .command(capability, Some(&term))
                    .unwrap_or_else(|| panic!("{:?} {:?}", adapter.family(), capability));
                assert!(!recipe.program.is_empty());
                assert_eq!(
                    recipe.elevated,
                    capability.mutates(),
                    "{:?} {:?} elevation does not match its policy",
                    adapter.family(),
                    capability
                );
                if capability.takes_term() {
                    assert_eq!(
                        recipe.args.last().map(String::as_str),
                        Some("ripgrep"),
                        "{:?} {:?} must take the package name last",
                        adapter.family(),
                        capability
                    );
                }
            }
        }
    }

    #[test]
    fn capabilities_reflect_what_is_installed() {
        let full = detect_with(&[], machine(&["dnf", "rpm", "pkexec"])).expect("an adapter");
        assert_eq!(full.capabilities(), Capability::ALL.to_vec());

        let bare = detect_with(&[], machine(&["dnf"])).expect("an adapter");
        assert_eq!(
            bare.capabilities(),
            vec![
                Capability::Search,
                Capability::Metadata,
                Capability::Updates
            ]
        );
    }
}
