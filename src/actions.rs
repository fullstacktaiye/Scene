//! Typed actions and their observable lifecycle.
//!
//! Search results carry these values, not shell strings. Registered process
//! actions are dispatched through `system`, where timeout, output and
//! cancellation are enforced.

use std::io::ErrorKind;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use crate::system::{self, CancellationToken, CommandSpec, ProcessError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionPolicy {
    /// A process whose output is read, bounded and reported to the user.
    ReadOnly,
    /// A graphical program that Scene starts but does not wait on.
    Detached,
    /// A durable system change. The UI must obtain confirmation first.
    Mutating,
}

#[derive(Clone, Debug)]
pub struct Confirmation {
    pub summary: String,
    pub target: String,
}

#[derive(Clone, Debug)]
pub struct ProcessAction {
    pub id: String,
    pub title: String,
    pub spec: CommandSpec,
    pub policy: ExecutionPolicy,
    pub confirmation: Option<Confirmation>,
}

impl ProcessAction {
    pub fn read_only(id: impl Into<String>, title: impl Into<String>, spec: CommandSpec) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            spec,
            policy: ExecutionPolicy::ReadOnly,
            confirmation: None,
        }
    }

    pub fn detached(id: impl Into<String>, title: impl Into<String>, spec: CommandSpec) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            spec,
            policy: ExecutionPolicy::Detached,
            confirmation: None,
        }
    }

    /// A durable system change. The confirmation is not optional here: the
    /// type system, not a caller's diligence, is what keeps a mutation from
    /// reaching [`start`].
    pub fn mutating(
        id: impl Into<String>,
        title: impl Into<String>,
        spec: CommandSpec,
        confirmation: Confirmation,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            spec,
            policy: ExecutionPolicy::Mutating,
            confirmation: Some(confirmation),
        }
    }
}

/// What executing a result actually does.
#[derive(Clone, Debug)]
pub enum Action {
    /// Start an installed application through the desktop's own application
    /// model, so startup notification and window activation work.
    Launch { app: gio::AppInfo },
    /// Hand a path or URI to the desktop's default handler.
    Open { target: String },
    /// A fixed process registered by an integration.
    Process { action: ProcessAction },
    /// Say something in the launcher without touching the system.
    Message { text: String },
    /// Close Scene.
    Quit,
}

/// The outcome in the shape the UI needs to render it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Pending(String),
    AwaitingConfirmation(String),
    Succeeded(String),
    Reported(String),
    Unavailable(String),
    Failed(String),
    Cancelled(String),
    TimedOut(String),
    Quit,
}

/// A process in flight. The UI owns this handle and may cancel it at any time.
pub struct RunningAction {
    cancellation: CancellationToken,
    receiver: Receiver<Outcome>,
}

impl RunningAction {
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }

    pub fn try_finish(&self) -> Option<Outcome> {
        self.receiver.try_recv().ok()
    }
}

pub enum StartedAction {
    Immediate(Outcome),
    Running(RunningAction),
}

/// Execute an action without blocking GTK. Only registered process actions
/// run in a worker; desktop launches remain immediate desktop requests.
pub fn start(action: &Action) -> StartedAction {
    let Action::Process { action } = action else {
        return StartedAction::Immediate(execute(action));
    };
    if action.policy == ExecutionPolicy::Mutating {
        return StartedAction::Immediate(Outcome::Failed(
            "Mutating actions must be confirmed before they are started.".into(),
        ));
    }
    start_process(action)
}

/// Start a previously confirmed mutating action. This is deliberately a
/// separate call so only the confirmation interaction can cross that boundary.
pub fn start_confirmed(action: &Action) -> StartedAction {
    let Action::Process { action } = action else {
        return StartedAction::Immediate(Outcome::Failed(
            "Only a registered process action can require confirmation.".into(),
        ));
    };
    if action.policy != ExecutionPolicy::Mutating || action.confirmation.is_none() {
        return StartedAction::Immediate(Outcome::Failed(
            "This action is not awaiting confirmation.".into(),
        ));
    }
    start_process(action)
}

fn start_process(action: &ProcessAction) -> StartedAction {
    if action.policy == ExecutionPolicy::Detached {
        return StartedAction::Immediate(detached(action));
    }

    let cancellation = CancellationToken::new();
    let worker_cancellation = cancellation.clone();
    let worker_action = action.clone();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(run_process(&worker_action, &worker_cancellation));
    });
    StartedAction::Running(RunningAction {
        cancellation,
        receiver,
    })
}

/// This synchronous entry point is useful to non-UI callers and focused tests.
/// The GTK surface calls [`start`] so a read-only command never blocks it.
pub fn execute(action: &Action) -> Outcome {
    match action {
        Action::Launch { app } => launch(app),
        Action::Open { target } => open(target),
        Action::Process { action } => {
            if action.policy == ExecutionPolicy::Mutating {
                Outcome::AwaitingConfirmation(confirmation_text(action))
            } else if action.policy == ExecutionPolicy::Detached {
                detached(action)
            } else {
                run_process(action, &CancellationToken::new())
            }
        }
        Action::Message { text } => Outcome::Reported(text.clone()),
        Action::Quit => Outcome::Quit,
    }
}

pub fn requires_confirmation(action: &Action) -> bool {
    matches!(action, Action::Process { action } if action.policy == ExecutionPolicy::Mutating && action.confirmation.is_some())
}

pub fn confirmation_text(action: &ProcessAction) -> String {
    let confirmation = action
        .confirmation
        .as_ref()
        .expect("mutating action requires confirmation metadata");
    format!(
        "{} Target: {}. Press Enter to confirm or Escape to cancel.",
        confirmation.summary, confirmation.target
    )
}

fn run_process(action: &ProcessAction, cancellation: &CancellationToken) -> Outcome {
    debug_assert!(
        !action.id.is_empty(),
        "registered actions require stable identifiers"
    );
    match system::run(&action.spec, cancellation) {
        Ok(output) => {
            let mut text = output.stdout;
            if !output.stderr.is_empty() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&output.stderr);
            }
            if text.is_empty() {
                text = format!("{} completed", action.title);
            }
            if output.truncated {
                text.push_str("\nOutput was truncated.");
            }
            Outcome::Reported(text)
        }
        Err(ProcessError::Unavailable(program)) => {
            Outcome::Unavailable(format!("{program} is not installed, or not on PATH"))
        }
        Err(ProcessError::PermissionDenied(program)) => {
            Outcome::Failed(format!("Not permitted to run {program}"))
        }
        Err(ProcessError::TimedOut(timeout)) => Outcome::TimedOut(format!(
            "{} exceeded its {} second limit",
            action.title,
            timeout.as_secs_f32()
        )),
        Err(ProcessError::Cancelled) => {
            Outcome::Cancelled(format!("{} was cancelled", action.title))
        }
        Err(ProcessError::NonZero { status, output }) => {
            let detail = if output.stderr.is_empty() {
                output.stdout
            } else {
                output.stderr
            };
            let suffix = if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            };
            Outcome::Failed(format!(
                "{} exited with status {}{suffix}",
                action.title,
                status.map_or_else(|| "unknown".into(), |code| code.to_string())
            ))
        }
        Err(ProcessError::Spawn(message)) => Outcome::Failed(message),
    }
}

/// Starts a graphical program without giving it input or inheriting output.
fn detached(action: &ProcessAction) -> Outcome {
    match Command::new(&action.spec.program)
        .args(&action.spec.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            thread::spawn(move || {
                let _ = child.wait();
            });
            Outcome::Succeeded(format!("Started {}", action.title))
        }
        Err(error) => Outcome::from(error, &action.spec.program),
    }
}

/// Hands the application back to the desktop to start. The launch context
/// carries the activation token, without which the new window would open
/// behind Scene on Wayland.
fn launch(app: &gio::AppInfo) -> Outcome {
    let name = app.display_name();
    let context = gdk::Display::default().map(|display| display.app_launch_context());
    match app.launch(&[], context.as_ref()) {
        Ok(()) => Outcome::Succeeded(format!("Opened {name}")),
        Err(error) => Outcome::Failed(format!("Could not open {name}: {error}")),
    }
}

fn run(program: &str, args: &[String]) -> Outcome {
    let action = ProcessAction::detached(
        "legacy.run",
        program,
        CommandSpec::read_only(program, args.iter().cloned()),
    );
    detached(&action)
}

/// `mailto:` and `tel:` are URIs with no "//", so ask GLib rather than
/// pattern-matching the string.
fn is_uri(target: &str) -> bool {
    glib::Uri::peek_scheme(target).is_some()
}

fn open(target: &str) -> Outcome {
    if !is_uri(target) && !Path::new(target).exists() {
        return Outcome::Unavailable(format!("{target} does not exist"));
    }
    match run("xdg-open", &[target.to_string()]) {
        Outcome::Succeeded(_) => Outcome::Succeeded(format!("Opened {target}")),
        other => other,
    }
}

impl Outcome {
    fn from(error: std::io::Error, program: &str) -> Self {
        match error.kind() {
            ErrorKind::NotFound => {
                Outcome::Unavailable(format!("{program} is not installed, or not on PATH"))
            }
            ErrorKind::PermissionDenied => {
                Outcome::Failed(format!("Not permitted to run {program}"))
            }
            _ => Outcome::Failed(format!("Could not start {program}: {error}")),
        }
    }

    pub fn should_dismiss(&self) -> bool {
        matches!(self, Outcome::Succeeded(_))
    }

    pub fn message(&self) -> &str {
        match self {
            Outcome::Pending(message)
            | Outcome::AwaitingConfirmation(message)
            | Outcome::Succeeded(message)
            | Outcome::Reported(message)
            | Outcome::Unavailable(message)
            | Outcome::Failed(message)
            | Outcome::Cancelled(message)
            | Outcome::TimedOut(message) => message,
            Outcome::Quit => "",
        }
    }

    pub fn prefix(&self) -> &'static str {
        match self {
            Outcome::Pending(_) => "Working",
            Outcome::AwaitingConfirmation(_) => "Confirm",
            Outcome::Succeeded(_) => "Done",
            Outcome::Reported(_) => "Scene",
            Outcome::Unavailable(_) => "Unavailable",
            Outcome::Failed(_) => "Failed",
            Outcome::Cancelled(_) => "Cancelled",
            Outcome::TimedOut(_) => "Timed out",
            Outcome::Quit => "",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Outcome::Pending(_) => "process-working-symbolic",
            Outcome::AwaitingConfirmation(_) => "dialog-warning-symbolic",
            Outcome::Succeeded(_) => "emblem-ok-symbolic",
            Outcome::Reported(_) => "dialog-information-symbolic",
            Outcome::Unavailable(_) => "action-unavailable-symbolic",
            Outcome::Failed(_) => "dialog-error-symbolic",
            Outcome::Cancelled(_) => "process-stop-symbolic",
            Outcome::TimedOut(_) => "alarm-symbolic",
            Outcome::Quit => "",
        }
    }

    pub fn tone(&self) -> &'static str {
        match self {
            Outcome::Pending(_) | Outcome::Reported(_) => "info",
            Outcome::AwaitingConfirmation(_) | Outcome::Unavailable(_) => "warn",
            Outcome::Succeeded(_) => "ok",
            Outcome::Failed(_) | Outcome::TimedOut(_) => "error",
            Outcome::Cancelled(_) => "info",
            Outcome::Quit => "info",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_missing_program_is_unavailable_rather_than_failed() {
        let outcome = run("scene-no-such-binary-for-tests", &[]);
        assert!(matches!(outcome, Outcome::Unavailable(_)), "{outcome:?}");
        assert!(!outcome.should_dismiss());
    }

    #[test]
    fn opening_a_missing_path_names_the_path() {
        let outcome = execute(&Action::Open {
            target: "/scene/no/such/path".into(),
        });
        assert!(
            matches!(outcome, Outcome::Unavailable(message) if message.contains("/scene/no/such/path"))
        );
    }

    #[test]
    fn a_uri_without_a_double_slash_is_still_a_uri() {
        assert!(is_uri("mailto:someone@example.com"));
        assert!(is_uri("tel:+441234567890"));
        assert!(is_uri("https://example.com"));
        assert!(!is_uri("/home/someone/notes.txt"));
    }

    #[test]
    fn a_mutating_action_cannot_start_without_confirmation() {
        let action = Action::Process {
            action: ProcessAction::mutating(
                "test.mutate",
                "Change something",
                CommandSpec::read_only("false", [] as [&str; 0]),
                Confirmation {
                    summary: "This changes something.".into(),
                    target: "test target".into(),
                },
            ),
        };
        assert!(requires_confirmation(&action));
        assert!(matches!(
            start(&action),
            StartedAction::Immediate(Outcome::Failed(_))
        ));
        assert!(matches!(execute(&action), Outcome::AwaitingConfirmation(_)));
    }

    #[test]
    fn every_outcome_states_itself_in_words() {
        for outcome in [
            Outcome::Pending(String::new()),
            Outcome::AwaitingConfirmation(String::new()),
            Outcome::Succeeded(String::new()),
            Outcome::Reported(String::new()),
            Outcome::Unavailable(String::new()),
            Outcome::Failed(String::new()),
            Outcome::Cancelled(String::new()),
            Outcome::TimedOut(String::new()),
        ] {
            assert!(!outcome.prefix().is_empty(), "{outcome:?}");
            assert!(!outcome.icon().is_empty(), "{outcome:?}");
        }
    }

    #[test]
    fn process_actions_keep_a_fixed_timeout() {
        let action = ProcessAction::read_only(
            "test",
            "Test",
            CommandSpec::read_only("test", [] as [&str; 0]).with_timeout(Duration::from_secs(2)),
        );
        assert_eq!(action.spec.timeout, Duration::from_secs(2));
    }
}
