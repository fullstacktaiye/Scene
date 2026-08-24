//! Typed actions and their observable lifecycle.
//!
//! Search results carry these values, not shell strings. Registered process
//! actions are dispatched through `system`, where timeout, output and
//! cancellation are enforced.

use std::io::ErrorKind;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use gio_unix::DesktopAppInfo;
use glib::variant::ToVariant;
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

/// How long Scene watches a program it started before it stops looking.
///
/// A graphical program is still running when this elapses, and that is the
/// answer. A program that dies inside the window — a missing library, an
/// argument the tool rejects — reports its real exit status instead of a
/// success Scene never observed. Past the window Scene is not watching, and
/// [`Outcome::Started`] says exactly that rather than claiming success.
pub const START_WATCH: Duration = Duration::from_millis(400);

#[derive(Clone, Debug)]
pub struct Confirmation {
    pub summary: String,
    pub target: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bus {
    Session,
    System,
}

#[derive(Clone, Debug)]
pub enum DbusArguments {
    None,
    String(String),
    Bool(bool),
    StringPair(String, String),
    DoubleU32(f64, u32),
}

#[derive(Clone, Debug)]
pub struct DbusAction {
    pub id: String,
    pub title: String,
    pub bus: Bus,
    pub service: String,
    pub path: String,
    pub interface: String,
    pub method: String,
    pub arguments: DbusArguments,
    pub confirmation: Option<Confirmation>,
    /// False for fire-and-forget APIs such as KRunner's `Run`, which return
    /// no execution result and therefore cannot honestly report success.
    pub observable: bool,
}

#[derive(Clone, Debug)]
pub struct SignalAction {
    pub id: String,
    pub title: String,
    pub pid: u32,
    pub signal: i32,
    pub confirmation: Confirmation,
}

#[derive(Clone, Debug)]
pub struct ProcessAction {
    pub id: String,
    pub title: String,
    pub spec: CommandSpec,
    pub policy: ExecutionPolicy,
    pub confirmation: Option<Confirmation>,
    pub history_entry: Option<String>,
}

impl ProcessAction {
    pub fn read_only(id: impl Into<String>, title: impl Into<String>, spec: CommandSpec) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            spec,
            policy: ExecutionPolicy::ReadOnly,
            confirmation: None,
            history_entry: None,
        }
    }

    pub fn detached(id: impl Into<String>, title: impl Into<String>, spec: CommandSpec) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            spec,
            policy: ExecutionPolicy::Detached,
            confirmation: None,
            history_entry: None,
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
            history_entry: None,
        }
    }

    pub fn with_history(mut self, entry: impl Into<String>) -> Self {
        self.history_entry = Some(entry.into());
        self
    }
}

/// What executing a result actually does.
#[derive(Clone, Debug)]
pub enum Action {
    /// Start an installed application through the desktop's own application
    /// model, so startup notification and window activation work.
    Launch { app: gio::AppInfo },
    /// Activate one of a desktop entry's declared additional operations.
    DesktopLaunch { app: DesktopAppInfo, name: String },
    /// Hand a path or URI to the desktop's default handler.
    Open { target: String },
    /// A fixed process registered by an integration.
    Process { action: ProcessAction },
    /// A typed desktop/system call. Service, object, interface, method and
    /// argument shape are frozen before the result is displayed.
    Dbus { action: DbusAction },
    /// Signal one user-owned process after a PID-naming confirmation.
    Signal { action: SignalAction },
    /// Copy a provider answer without launching an external process.
    Copy { text: String, label: String },
    /// Say something in the launcher without touching the system.
    Message { text: String },
    /// Navigate to Scene's own settings surface.
    ShowSettings,
    /// Close Scene.
    Quit,
}

/// The outcome in the shape the UI needs to render it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Pending(String),
    AwaitingConfirmation(String),
    /// Scene watched the work finish successfully.
    Succeeded(String),
    /// The program started and Scene is no longer watching it: a desktop
    /// launch it has no handle on, or a program still running when
    /// [`START_WATCH`] elapsed. Deliberately not [`Outcome::Succeeded`],
    /// which is reserved for an outcome Scene actually observed.
    Started(String),
    Reported(String),
    Unavailable(String),
    Failed(String),
    Cancelled(String),
    TimedOut(String),
    ShowSettings,
    Quit,
}

/// A process in flight. The UI owns this handle, and may cancel it when the
/// action is one Scene is waiting on.
pub struct RunningAction {
    cancellation: CancellationToken,
    receiver: Receiver<Outcome>,
    /// A watched launch is not cancellable. Escape closes the launcher; it
    /// does not kill the program the user has just started.
    cancellable: bool,
}

impl RunningAction {
    pub fn cancel(&self) {
        if self.cancellable {
            self.cancellation.cancel();
        }
    }

    pub fn is_cancellable(&self) -> bool {
        self.cancellable
    }

    pub fn try_finish(&self) -> Option<Outcome> {
        self.receiver.try_recv().ok()
    }
}

pub enum StartedAction {
    Immediate(Outcome),
    Running(RunningAction),
}

/// Execute an action without blocking GTK. Anything that starts a process —
/// a registered command or an `xdg-open` hand-off — runs in a worker, because
/// even a detached start is watched for [`START_WATCH`] before Scene reports
/// what happened. A desktop launch stays an immediate desktop request.
pub fn start(action: &Action) -> StartedAction {
    match action {
        Action::Process { action } if action.policy == ExecutionPolicy::Mutating => {
            StartedAction::Immediate(Outcome::Failed(
                "Mutating actions must be confirmed before they are started.".into(),
            ))
        }
        Action::Process { action } => start_process(action),
        Action::Dbus { action } if action.confirmation.is_some() => StartedAction::Immediate(
            Outcome::Failed("This desktop action must be confirmed before it is started.".into()),
        ),
        Action::Dbus { action } => start_dbus(action),
        Action::Signal { .. } => StartedAction::Immediate(Outcome::Failed(
            "A process signal must be confirmed before it is sent.".into(),
        )),
        Action::Open { target } => match open_action(target) {
            Ok(action) => start_process(&action),
            Err(outcome) => StartedAction::Immediate(outcome),
        },
        _ => StartedAction::Immediate(execute(action)),
    }
}

/// Start a previously confirmed mutating action. This is deliberately a
/// separate call so only the confirmation interaction can cross that boundary.
pub fn start_confirmed(action: &Action) -> StartedAction {
    match action {
        Action::Process { action }
            if action.policy == ExecutionPolicy::Mutating && action.confirmation.is_some() =>
        {
            start_process(action)
        }
        Action::Dbus { action } if action.confirmation.is_some() => start_dbus(action),
        Action::Signal { action } => start_signal(action),
        _ => StartedAction::Immediate(Outcome::Failed(
            "This action is not awaiting confirmation.".into(),
        )),
    }
}

fn start_process(action: &ProcessAction) -> StartedAction {
    // A detached program is watched, not waited on: cancelling it would mean
    // killing something the user asked to start, so the token never reaches it.
    let cancellable = action.policy != ExecutionPolicy::Detached;
    let cancellation = CancellationToken::new();
    let worker_cancellation = cancellation.clone();
    let worker_action = action.clone();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let outcome = if worker_action.policy == ExecutionPolicy::Detached {
            detached(&worker_action)
        } else {
            run_process(&worker_action, &worker_cancellation)
        };
        let _ = sender.send(outcome);
    });
    StartedAction::Running(RunningAction {
        cancellation,
        receiver,
        cancellable,
    })
}

/// This synchronous entry point is useful to non-UI callers and focused tests.
/// The GTK surface calls [`start`] so a read-only command never blocks it.
pub fn execute(action: &Action) -> Outcome {
    match action {
        Action::Launch { app } => launch(app),
        Action::DesktopLaunch { app, name } => {
            let context = gdk::Display::default().map(|display| display.app_launch_context());
            app.launch_action(name, context.as_ref());
            Outcome::Started(format!("Started {}", app.display_name()))
        }
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
        Action::Dbus { action } => {
            if let Some(confirmation) = &action.confirmation {
                Outcome::AwaitingConfirmation(confirmation_for(confirmation))
            } else {
                run_dbus(action)
            }
        }
        Action::Signal { action } => {
            Outcome::AwaitingConfirmation(confirmation_for(&action.confirmation))
        }
        Action::Copy { text, label } => match gdk::Display::default() {
            Some(display) => {
                display.clipboard().set_text(text);
                Outcome::Reported(format!("Copied {label}"))
            }
            None => Outcome::Unavailable("No desktop clipboard is available.".into()),
        },
        Action::Message { text } => Outcome::Reported(text.clone()),
        Action::ShowSettings => Outcome::ShowSettings,
        Action::Quit => Outcome::Quit,
    }
}

pub fn requires_confirmation(action: &Action) -> bool {
    matches!(action, Action::Process { action } if action.policy == ExecutionPolicy::Mutating && action.confirmation.is_some())
        || matches!(action, Action::Dbus { action } if action.confirmation.is_some())
        || matches!(action, Action::Signal { .. })
}

pub fn action_confirmation_text(action: &Action) -> Option<String> {
    match action {
        Action::Process { action } if action.confirmation.is_some() => {
            Some(confirmation_text(action))
        }
        Action::Dbus { action } => action.confirmation.as_ref().map(confirmation_for),
        Action::Signal { action } => Some(confirmation_for(&action.confirmation)),
        _ => None,
    }
}

pub fn confirmation_text(action: &ProcessAction) -> String {
    let confirmation = action
        .confirmation
        .as_ref()
        .expect("mutating action requires confirmation metadata");
    confirmation_for(confirmation)
}

fn confirmation_for(confirmation: &Confirmation) -> String {
    format!(
        "{} Target: {}. Press Enter to confirm or Escape to cancel.",
        confirmation.summary, confirmation.target
    )
}

fn start_dbus(action: &DbusAction) -> StartedAction {
    let (sender, receiver) = mpsc::channel();
    let action = action.clone();
    thread::spawn(move || {
        let _ = sender.send(run_dbus(&action));
    });
    StartedAction::Running(RunningAction {
        cancellation: CancellationToken::new(),
        receiver,
        cancellable: false,
    })
}

fn run_dbus(action: &DbusAction) -> Outcome {
    debug_assert!(
        !action.id.is_empty(),
        "D-Bus actions require stable identifiers"
    );
    let bus = match action.bus {
        Bus::Session => gio::BusType::Session,
        Bus::System => gio::BusType::System,
    };
    let connection = match gio::bus_get_sync(bus, gio::Cancellable::NONE) {
        Ok(connection) => connection,
        Err(error) => {
            return Outcome::Unavailable(format!("{} is unavailable: {error}", action.title));
        }
    };
    let parameters = match &action.arguments {
        DbusArguments::None => None,
        DbusArguments::String(value) => Some((value.as_str(),).to_variant()),
        DbusArguments::Bool(value) => Some((*value,).to_variant()),
        DbusArguments::StringPair(first, second) => {
            Some((first.as_str(), second.as_str()).to_variant())
        }
        DbusArguments::DoubleU32(value, flags) => Some((*value, *flags).to_variant()),
    };
    match connection.call_sync(
        Some(&action.service),
        &action.path,
        &action.interface,
        &action.method,
        parameters.as_ref(),
        None,
        gio::DBusCallFlags::NONE,
        5_000,
        gio::Cancellable::NONE,
    ) {
        Ok(_) if action.observable => Outcome::Succeeded(action.title.clone()),
        Ok(_) => Outcome::Started(action.title.clone()),
        Err(error) => Outcome::Failed(format!("{}: {error}", action.title)),
    }
}

fn start_signal(action: &SignalAction) -> StartedAction {
    debug_assert!(
        !action.id.is_empty(),
        "signal actions require stable identifiers"
    );
    let (sender, receiver) = mpsc::channel();
    let action = action.clone();
    thread::spawn(move || {
        let pid = nix::unistd::Pid::from_raw(action.pid as i32);
        let outcome = match nix::sys::signal::kill(
            pid,
            nix::sys::signal::Signal::try_from(action.signal).ok(),
        ) {
            Ok(()) => {
                let started = Instant::now();
                while started.elapsed() < Duration::from_millis(500) {
                    if matches!(
                        nix::sys::signal::kill(pid, None),
                        Err(nix::errno::Errno::ESRCH)
                    ) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                if matches!(
                    nix::sys::signal::kill(pid, None),
                    Err(nix::errno::Errno::ESRCH)
                ) {
                    Outcome::Succeeded(format!("PID {} stopped", action.pid))
                } else {
                    Outcome::Reported(format!(
                        "Signal {} was delivered to PID {}, which is still present",
                        action.signal, action.pid
                    ))
                }
            }
            Err(error) => Outcome::Failed(format!("Could not signal PID {}: {error}", action.pid)),
        };
        let _ = sender.send(outcome);
    });
    StartedAction::Running(RunningAction {
        cancellation: CancellationToken::new(),
        receiver,
        cancellable: false,
    })
}

fn run_process(action: &ProcessAction, cancellation: &CancellationToken) -> Outcome {
    debug_assert!(
        !action.id.is_empty(),
        "registered actions require stable identifiers"
    );
    match system::run(&action.spec, cancellation) {
        Ok(output) => {
            if let Some(entry) = &action.history_entry {
                record_command_history(entry);
            }
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

pub fn command_history() -> Vec<String> {
    let Some(path) = command_history_path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut lines = text.lines();
    if lines.next() != Some("scene-command-history 1") {
        return Vec::new();
    }
    lines
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

pub fn clear_command_history() -> std::io::Result<()> {
    let Some(path) = command_history_path() else {
        return Ok(());
    };
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn record_command_history(entry: &str) {
    let Some(path) = command_history_path() else {
        return;
    };
    let mut entries = command_history();
    entries.retain(|existing| existing != entry);
    entries.insert(0, entry.to_string());
    entries.truncate(100);
    let mut text = String::from("scene-command-history 1\n");
    for entry in entries {
        if let Ok(line) = serde_json::to_string(&entry) {
            text.push_str(&line);
            text.push('\n');
        }
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    if std::fs::write(&temporary, text).is_ok() {
        let _ = std::fs::rename(temporary, path);
    }
}

fn command_history_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|home| std::path::PathBuf::from(home).join(".local").join("state"))
        })?;
    Some(base.join("scene").join("command-history"))
}

/// Starts a program without giving it input or inheriting output, then
/// watches it for as long as [`START_WATCH`].
fn detached(action: &ProcessAction) -> Outcome {
    match Command::new(&action.spec.program)
        .args(&action.spec.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => watch(child, &action.title),
        Err(error) => Outcome::from(error, &action.spec.program),
    }
}

/// Reports what the watch actually saw, and nothing more. A program that ends
/// inside the window is reported by its exit status; one still running when
/// the window closes is reported as started, not as succeeded.
fn watch(mut child: Child, title: &str) -> Outcome {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                return Outcome::Succeeded(format!("{title} finished"));
            }
            Ok(Some(status)) => {
                return Outcome::Failed(format!(
                    "{title} exited with status {}",
                    status
                        .code()
                        .map_or_else(|| "unknown".into(), |code| code.to_string())
                ));
            }
            Ok(None) => {
                if started.elapsed() >= START_WATCH {
                    // Nothing waits on it from here, so it is reaped in the
                    // background rather than left behind as a zombie.
                    thread::spawn(move || {
                        let _ = child.wait();
                    });
                    return Outcome::Started(format!("{title} is running"));
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Outcome::Failed(format!("Could not watch {title}: {error}")),
        }
    }
}

/// Hands the application back to the desktop to start. The launch context
/// carries the activation token, without which the new window would open
/// behind Scene on Wayland.
///
/// The corollary is a real limit: the desktop owns the application from here,
/// Scene holds no handle on it and has no exit status to read, so the outcome
/// is [`Outcome::Started`] rather than a success it never observed. That limit
/// is stated in the launcher's own "What Scene Reports" result.
fn launch(app: &gio::AppInfo) -> Outcome {
    let name = app.display_name();
    let context = gdk::Display::default().map(|display| display.app_launch_context());
    match app.launch(&[], context.as_ref()) {
        Ok(()) => Outcome::Started(format!("{name} was handed to the desktop")),
        Err(error) => Outcome::Failed(format!("Could not open {name}: {error}")),
    }
}

/// `mailto:` and `tel:` are URIs with no "//", so ask GLib rather than
/// pattern-matching the string.
fn is_uri(target: &str) -> bool {
    glib::Uri::peek_scheme(target).is_some()
}

/// The `xdg-open` hand-off for a path or URI, or the reason there is none. A
/// path that does not exist is answered here rather than passed on.
fn open_action(target: &str) -> Result<ProcessAction, Outcome> {
    if !is_uri(target) && !Path::new(target).exists() {
        return Err(Outcome::Unavailable(format!("{target} does not exist")));
    }
    Ok(ProcessAction::detached(
        "open",
        format!("Opening {target}"),
        CommandSpec::read_only("xdg-open", [target.to_string()]),
    ))
}

fn open(target: &str) -> Outcome {
    match open_action(target) {
        Ok(action) => detached(&action),
        Err(outcome) => outcome,
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
        matches!(self, Outcome::Succeeded(_) | Outcome::Started(_))
    }

    pub fn message(&self) -> &str {
        match self {
            Outcome::Pending(message)
            | Outcome::AwaitingConfirmation(message)
            | Outcome::Succeeded(message)
            | Outcome::Started(message)
            | Outcome::Reported(message)
            | Outcome::Unavailable(message)
            | Outcome::Failed(message)
            | Outcome::Cancelled(message)
            | Outcome::TimedOut(message) => message,
            Outcome::ShowSettings | Outcome::Quit => "",
        }
    }

    pub fn prefix(&self) -> &'static str {
        match self {
            Outcome::Pending(_) => "Working",
            Outcome::AwaitingConfirmation(_) => "Confirm",
            Outcome::Succeeded(_) => "Done",
            Outcome::Started(_) => "Started",
            Outcome::Reported(_) => "Scene",
            Outcome::Unavailable(_) => "Unavailable",
            Outcome::Failed(_) => "Failed",
            Outcome::Cancelled(_) => "Cancelled",
            Outcome::TimedOut(_) => "Timed out",
            Outcome::ShowSettings | Outcome::Quit => "",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Outcome::Pending(_) => "process-working-symbolic",
            Outcome::AwaitingConfirmation(_) => "dialog-warning-symbolic",
            Outcome::Succeeded(_) => "emblem-ok-symbolic",
            Outcome::Started(_) => "media-playback-start-symbolic",
            Outcome::Reported(_) => "dialog-information-symbolic",
            Outcome::Unavailable(_) => "action-unavailable-symbolic",
            Outcome::Failed(_) => "dialog-error-symbolic",
            Outcome::Cancelled(_) => "process-stop-symbolic",
            Outcome::TimedOut(_) => "alarm-symbolic",
            Outcome::ShowSettings | Outcome::Quit => "",
        }
    }

    pub fn tone(&self) -> &'static str {
        match self {
            Outcome::Pending(_) | Outcome::Reported(_) => "info",
            Outcome::AwaitingConfirmation(_) | Outcome::Unavailable(_) => "warn",
            Outcome::Succeeded(_) | Outcome::Started(_) => "ok",
            Outcome::Failed(_) | Outcome::TimedOut(_) => "error",
            Outcome::Cancelled(_) => "info",
            Outcome::ShowSettings | Outcome::Quit => "info",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detach(program: &str, args: &[&str]) -> Outcome {
        detached(&ProcessAction::detached(
            "test.detached",
            "Test program",
            CommandSpec::read_only(program, args.iter().map(|argument| argument.to_string())),
        ))
    }

    /// Same `ETXTBSY` race as `system::tests::run_fake`: a fork while another
    /// test is still writing its fake program can see the file as busy. The
    /// executable is sound; only the timing is not.
    #[cfg(unix)]
    fn detach_fake(body: &str) -> Outcome {
        let program = crate::system::tests::fake_program(body);
        let mut outcome = detach(&program.to_string_lossy(), &[]);
        for _ in 0..50 {
            let busy =
                matches!(&outcome, Outcome::Failed(message) if message.contains("Text file busy"));
            if !busy {
                break;
            }
            thread::sleep(Duration::from_millis(10));
            outcome = detach(&program.to_string_lossy(), &[]);
        }
        std::fs::remove_file(program).expect("remove fake executable");
        outcome
    }

    #[test]
    fn a_missing_program_is_unavailable_rather_than_failed() {
        let outcome = detach("scene-no-such-binary-for-tests", &[]);
        assert!(matches!(outcome, Outcome::Unavailable(_)), "{outcome:?}");
        assert!(!outcome.should_dismiss());
    }

    #[cfg(unix)]
    #[test]
    fn a_launch_that_starts_and_then_fails_is_not_reported_as_success() {
        // The gap this closes: nothing used to wait on a detached program, so
        // one that started and died immediately still reported success.
        let outcome = detach_fake("exit 3");
        assert!(
            matches!(&outcome, Outcome::Failed(message) if message.contains("status 3")),
            "{outcome:?}"
        );
        assert!(!outcome.should_dismiss());
    }

    #[cfg(unix)]
    #[test]
    fn a_program_that_finishes_cleanly_inside_the_window_succeeded() {
        let outcome = detach_fake("exit 0");
        assert!(matches!(outcome, Outcome::Succeeded(_)), "{outcome:?}");
        assert!(outcome.should_dismiss());
    }

    #[cfg(unix)]
    #[test]
    fn a_program_still_running_when_the_window_closes_is_started_not_succeeded() {
        // What a graphical program does. Scene stopped watching, so it says
        // the program started rather than that it succeeded.
        let started = Instant::now();
        let outcome = detach_fake("sleep 5");
        assert!(matches!(outcome, Outcome::Started(_)), "{outcome:?}");
        assert!(started.elapsed() >= START_WATCH, "the watch ended early");
        assert!(outcome.should_dismiss());
    }

    #[test]
    fn started_is_a_different_answer_from_succeeded() {
        let started = Outcome::Started("Firefox was handed to the desktop".into());
        let succeeded = Outcome::Succeeded("Firefox finished".into());
        assert_ne!(started.prefix(), succeeded.prefix());
        assert_ne!(started.icon(), succeeded.icon());
        // Both close the launcher; only one of them claims an observed result.
        assert!(started.should_dismiss() && succeeded.should_dismiss());
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
            Outcome::Started(String::new()),
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
