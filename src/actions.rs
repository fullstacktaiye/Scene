//! Typed actions.
//!
//! A search result never carries a shell string. It carries one of the
//! variants below, and `execute` is the only place that can start work.

use std::io::ErrorKind;
use std::path::Path;
use std::process::{Command, Stdio};

/// What executing a result actually does.
#[derive(Clone, Debug)]
pub enum Action {
    /// Run a program, detached from Scene.
    Run { program: String, args: Vec<String> },
    /// Hand a path or URI to the desktop's default handler.
    Open { target: String },
    /// Say something in the launcher without touching the system.
    Message { text: String },
    /// Close Scene.
    Quit,
}

/// The outcome of an execution, in the shape the UI needs to render it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It ran. The launcher should get out of the way.
    Succeeded(String),
    /// It ran and has something to say. The launcher stays open.
    Reported(String),
    /// Something it needs is missing, so nothing happened.
    Unavailable(String),
    /// It was attempted and failed.
    Failed(String),
    /// Scene should exit.
    Quit,
}

pub fn execute(action: &Action) -> Outcome {
    match action {
        Action::Run { program, args } => run(program, args),
        Action::Open { target } => open(target),
        Action::Message { text } => Outcome::Reported(text.clone()),
        Action::Quit => Outcome::Quit,
    }
}

fn run(program: &str, args: &[String]) -> Outcome {
    let spawned = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();

    match spawned {
        Ok(mut child) => {
            // Nothing waits on the child otherwise, and Scene outlives it.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            Outcome::Succeeded(format!("Started {program}"))
        }
        Err(e) => Outcome::from(e, program),
    }
}

fn open(target: &str) -> Outcome {
    let is_uri = target.contains("://");
    if !is_uri && !Path::new(target).exists() {
        return Outcome::Unavailable(format!("{target} does not exist"));
    }
    match run("xdg-open", &[target.to_string()]) {
        Outcome::Succeeded(_) => Outcome::Succeeded(format!("Opened {target}")),
        other => other,
    }
}

impl Outcome {
    fn from(e: std::io::Error, program: &str) -> Self {
        match e.kind() {
            ErrorKind::NotFound => {
                Outcome::Unavailable(format!("{program} is not installed, or not on PATH"))
            }
            ErrorKind::PermissionDenied => {
                Outcome::Failed(format!("Not permitted to run {program}"))
            }
            _ => Outcome::Failed(format!("Could not start {program}: {e}")),
        }
    }

    /// Whether the launcher should hide itself after this outcome.
    pub fn should_dismiss(&self) -> bool {
        matches!(self, Outcome::Succeeded(_))
    }

    pub fn message(&self) -> &str {
        match self {
            Outcome::Succeeded(m)
            | Outcome::Reported(m)
            | Outcome::Unavailable(m)
            | Outcome::Failed(m) => m,
            Outcome::Quit => "",
        }
    }

    /// A word the UI puts in front of the message, so the state is never
    /// carried by colour alone.
    pub fn prefix(&self) -> &'static str {
        match self {
            Outcome::Succeeded(_) => "Done",
            Outcome::Reported(_) => "Scene",
            Outcome::Unavailable(_) => "Unavailable",
            Outcome::Failed(_) => "Failed",
            Outcome::Quit => "",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Outcome::Succeeded(_) => "emblem-ok-symbolic",
            Outcome::Reported(_) => "dialog-information-symbolic",
            Outcome::Unavailable(_) => "action-unavailable-symbolic",
            Outcome::Failed(_) => "dialog-error-symbolic",
            Outcome::Quit => "",
        }
    }

    pub fn tone(&self) -> &'static str {
        match self {
            Outcome::Succeeded(_) => "ok",
            Outcome::Reported(_) => "info",
            Outcome::Unavailable(_) => "warn",
            Outcome::Failed(_) => "error",
            Outcome::Quit => "info",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_program_is_unavailable_rather_than_failed() {
        let outcome = execute(&Action::Run {
            program: "scene-no-such-binary-for-tests".into(),
            args: Vec::new(),
        });
        assert!(matches!(outcome, Outcome::Unavailable(_)), "{outcome:?}");
        assert!(!outcome.should_dismiss());
    }

    #[test]
    fn opening_a_missing_path_names_the_path() {
        let outcome = execute(&Action::Open {
            target: "/scene/no/such/path".into(),
        });
        match outcome {
            Outcome::Unavailable(message) => assert!(message.contains("/scene/no/such/path")),
            other => panic!("expected unavailable, got {other:?}"),
        }
    }

    #[test]
    fn a_message_keeps_the_launcher_open() {
        let outcome = execute(&Action::Message {
            text: "hello".into(),
        });
        assert_eq!(outcome, Outcome::Reported("hello".into()));
        assert!(!outcome.should_dismiss());
    }

    #[test]
    fn every_outcome_states_itself_in_words() {
        for outcome in [
            Outcome::Succeeded(String::new()),
            Outcome::Reported(String::new()),
            Outcome::Unavailable(String::new()),
            Outcome::Failed(String::new()),
        ] {
            assert!(!outcome.prefix().is_empty(), "{outcome:?}");
            assert!(!outcome.icon().is_empty(), "{outcome:?}");
        }
    }
}
