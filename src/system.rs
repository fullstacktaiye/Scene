//! Bounded subprocess execution for registered Scene actions.
//!
//! This is intentionally the only module that starts a generic executable.
//! Providers construct a fixed [`CommandSpec`]; neither the search query nor
//! the GTK surface is interpreted as a shell command.

use std::io::{ErrorKind, Read};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

/// A cancellation handle shared by the UI and a running action.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// The command is registered by an integration, never assembled from a query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub timeout: Duration,
    pub output_limit: usize,
}

impl CommandSpec {
    pub fn read_only(
        program: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            timeout: Duration::from_secs(3),
            output_limit: 16 * 1024,
        }
    }
}

/// The action's observable completion, including bounded captured output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessOutput {
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcessError {
    Unavailable(String),
    PermissionDenied(String),
    TimedOut(Duration),
    Cancelled,
    NonZero {
        status: Option<i32>,
        output: ProcessOutput,
    },
    Spawn(String),
}

pub type ProcessResult = Result<ProcessOutput, ProcessError>;

/// Run a known command with null input, captured bounded output and a timeout.
pub fn run(spec: &CommandSpec, cancellation: &CancellationToken) -> ProcessResult {
    if cancellation.is_cancelled() {
        return Err(ProcessError::Cancelled);
    }

    let mut child = Command::new(&spec.program)
        .args(&spec.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| match error.kind() {
            ErrorKind::NotFound => ProcessError::Unavailable(spec.program.clone()),
            ErrorKind::PermissionDenied => ProcessError::PermissionDenied(spec.program.clone()),
            _ => ProcessError::Spawn(format!("Could not start {}: {error}", spec.program)),
        })?;

    let stdout = child.stdout.take().expect("piped stdout is present");
    let stderr = child.stderr.take().expect("piped stderr is present");
    let limit = spec.output_limit;
    let stdout_reader = thread::spawn(move || read_limited(stdout, limit));
    let stderr_reader = thread::spawn(move || read_limited(stderr, limit));
    let started = Instant::now();

    let status = loop {
        if cancellation.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(ProcessError::Cancelled);
        }
        if started.elapsed() >= spec.timeout {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(ProcessError::TimedOut(spec.timeout));
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(ProcessError::Spawn(format!(
                    "Could not wait for {}: {error}",
                    spec.program
                )));
            }
        }
    };

    let (stdout, stdout_truncated) = stdout_reader.join().unwrap_or_default();
    let (stderr, stderr_truncated) = stderr_reader.join().unwrap_or_default();
    let output = ProcessOutput {
        stdout: String::from_utf8_lossy(&stdout).trim().to_string(),
        stderr: String::from_utf8_lossy(&stderr).trim().to_string(),
        truncated: stdout_truncated || stderr_truncated,
    };
    exit_result(status, output)
}

fn exit_result(status: ExitStatus, output: ProcessOutput) -> ProcessResult {
    if status.success() {
        Ok(output)
    } else {
        Err(ProcessError::NonZero {
            status: status.code(),
            output,
        })
    }
}

fn read_limited(mut reader: impl Read, limit: usize) -> (Vec<u8>, bool) {
    let mut output = Vec::new();
    let mut chunk = [0; 4096];
    let mut truncated = false;
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                let remaining = limit.saturating_sub(output.len());
                let kept = read.min(remaining);
                output.extend_from_slice(&chunk[..kept]);
                truncated |= kept < read;
            }
            Err(_) => break,
        }
    }
    (output, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cancelled_request_never_starts_a_process() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let result = run(
            &CommandSpec::read_only("scene-does-not-exist", [] as [&str; 0]),
            &cancellation,
        );
        assert_eq!(result, Err(ProcessError::Cancelled));
    }

    #[test]
    fn output_limit_keeps_only_the_configured_prefix() {
        let (output, truncated) = read_limited("abcdef".as_bytes(), 3);
        assert_eq!(output, b"abc");
        assert!(truncated);
    }

    #[cfg(unix)]
    fn fake_program(body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = std::env::temp_dir().join(format!(
            "scene-system-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time is after the Unix epoch")
                .as_nanos()
        ));
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write fake executable");
        let mut permissions = std::fs::metadata(&path)
            .expect("read fake executable metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&path, permissions).expect("make fake executable runnable");
        path
    }

    #[cfg(unix)]
    #[test]
    fn captures_stdout_and_stderr_from_a_registered_fake_program() {
        let program = fake_program("printf 'out'; printf 'err' >&2");
        let output = run(
            &CommandSpec::read_only(program.to_string_lossy(), [] as [&str; 0]),
            &CancellationToken::new(),
        )
        .expect("fake program succeeds");
        std::fs::remove_file(program).expect("remove fake executable");
        assert_eq!(output.stdout, "out");
        assert_eq!(output.stderr, "err");
    }

    #[cfg(unix)]
    #[test]
    fn a_running_fake_program_is_stopped_at_its_timeout() {
        let program = fake_program("sleep 1");
        let mut spec = CommandSpec::read_only(program.to_string_lossy(), [] as [&str; 0]);
        spec.timeout = Duration::from_millis(20);
        let result = run(&spec, &CancellationToken::new());
        std::fs::remove_file(program).expect("remove fake executable");
        assert_eq!(
            result,
            Err(ProcessError::TimedOut(Duration::from_millis(20)))
        );
    }
}
