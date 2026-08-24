//! Desktop-specific capabilities that Scene can observe without changing them.
//!
//! KDE owns global shortcut registration, recording, and conflict handling.
//! Scene reads that state and can open KDE's recorder, but it never edits the
//! desktop's configuration itself.

use std::path::{Path, PathBuf};

use crate::actions::{Action, ProcessAction};
use crate::system::{self, CommandSpec};

pub const FALLBACK_SHORTCUT: &str = "Meta+Space";

const SHORTCUT_GROUP: &str = "services][dev.scene.Scene.desktop";
const SHORTCUT_KEY: &str = "_launch";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesktopSupport {
    Kde {
        session_type: String,
    },
    Unsupported {
        desktop: String,
        session_type: String,
    },
}

impl DesktopSupport {
    pub fn summary(&self) -> String {
        match self {
            Self::Kde { session_type } => format!("KDE Plasma ({session_type})"),
            Self::Unsupported {
                desktop,
                session_type,
            } => format!("{desktop} ({session_type}); KDE shortcut integration unavailable"),
        }
    }

    pub fn is_kde(&self) -> bool {
        matches!(self, Self::Kde { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortcutStatus {
    pub desktop: DesktopSupport,
    pub active: Vec<String>,
    pub fallback: &'static str,
    pub recorder: Option<Recorder>,
}

impl ShortcutStatus {
    pub fn detect() -> Self {
        Inputs::from_environment().detect()
    }

    pub fn shortcut_summary(&self) -> String {
        if !self.desktop.is_kde() {
            return format!("Fallback: {} (not verified on this session)", self.fallback);
        }
        if self.active.is_empty() {
            format!(
                "No active shortcut observed; packaged fallback: {}",
                self.fallback
            )
        } else {
            format!("Active: {}", self.active.join(", "))
        }
    }

    pub fn recorder_action(&self) -> Option<Action> {
        let recorder = self.recorder.as_ref()?;
        Some(Action::Process {
            action: ProcessAction::detached(
                "settings.shortcuts.open",
                "KDE Shortcuts",
                CommandSpec::read_only(&recorder.program, recorder.args.clone()),
            ),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recorder {
    program: String,
    args: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CopilotStatus {
    #[default]
    NotTested,
    Waiting,
    BindableObserved,
    UnbindableObserved,
    ActivationObserved,
    NotObserved,
}

impl CopilotStatus {
    pub fn summary(self) -> &'static str {
        match self {
            Self::NotTested => "Not tested in this session",
            Self::Waiting => "Waiting for the Copilot key…",
            Self::BindableObserved => "Observed Meta+Shift+F23; KDE can bind this form",
            Self::UnbindableObserved => "Observed XF86Assistant; this KDE/Qt path cannot record it",
            Self::ActivationObserved => "Observed through Scene's KDE desktop action",
            Self::NotObserved => "No Copilot-key event was observed",
        }
    }
}

pub fn classify_copilot_key(key_name: &str, shift: bool, meta: bool) -> Option<CopilotStatus> {
    if key_name.eq_ignore_ascii_case("XF86Assistant") {
        Some(CopilotStatus::UnbindableObserved)
    } else if key_name.eq_ignore_ascii_case("F23") && shift && meta {
        Some(CopilotStatus::BindableObserved)
    } else {
        None
    }
}

struct Inputs {
    desktop: String,
    session_type: String,
    config_path: PathBuf,
}

impl Inputs {
    fn from_environment() -> Self {
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "Unknown".into());
        let session_type = std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into());
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .unwrap_or_else(|| PathBuf::from(".config"));
        Self {
            desktop,
            session_type,
            config_path: config_home.join("kglobalshortcutsrc"),
        }
    }

    fn detect(&self) -> ShortcutStatus {
        let kde = self
            .desktop
            .split(':')
            .any(|name| matches!(name.to_ascii_lowercase().as_str(), "kde" | "plasma"));
        let desktop = if kde {
            DesktopSupport::Kde {
                session_type: self.session_type.clone(),
            }
        } else {
            DesktopSupport::Unsupported {
                desktop: self.desktop.clone(),
                session_type: self.session_type.clone(),
            }
        };

        ShortcutStatus {
            active: if kde {
                read_shortcuts(&self.config_path)
            } else {
                Vec::new()
            },
            recorder: kde.then(detect_recorder).flatten(),
            desktop,
            fallback: FALLBACK_SHORTCUT,
        }
    }
}

fn read_shortcuts(path: &Path) -> Vec<String> {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let target = format!("[{SHORTCUT_GROUP}]");
    let mut in_target = false;
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_target = line == target;
            continue;
        }
        if in_target
            && let Some((key, value)) = line.split_once('=')
            && key.trim() == SHORTCUT_KEY
        {
            return value
                .replace("\\t", "\t")
                .split('\t')
                .map(str::trim)
                .filter(|shortcut| !shortcut.is_empty() && !shortcut.eq_ignore_ascii_case("none"))
                .map(str::to_string)
                .collect();
        }
    }
    Vec::new()
}

fn detect_recorder() -> Option<Recorder> {
    [("systemsettings", "kcm_keys"), ("kcmshell6", "kcm_keys")]
        .into_iter()
        .find(|(program, _)| system::executable_on_path(program))
        .map(|(program, page)| Recorder {
            program: program.into(),
            args: vec![page.into()],
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture(contents: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "scene-shortcuts-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        fs::write(&path, contents).expect("write shortcut fixture");
        path
    }

    #[test]
    fn kde_shortcuts_are_read_as_separate_active_bindings() {
        let path =
            fixture("[services][dev.scene.Scene.desktop]\n_launch=Meta+Space\\tMeta+Shift+F23\n");
        assert_eq!(read_shortcuts(&path), ["Meta+Space", "Meta+Shift+F23"]);
        fs::remove_file(path).expect("remove shortcut fixture");

        let path =
            fixture("[services][dev.scene.Scene.desktop]\n_launch=Meta+Space\tMeta+Shift+F23\n");
        assert_eq!(read_shortcuts(&path), ["Meta+Space", "Meta+Shift+F23"]);
        fs::remove_file(path).expect("remove shortcut fixture");
    }

    #[test]
    fn missing_or_disabled_shortcuts_are_unconfigured() {
        assert!(read_shortcuts(Path::new("/scene/no-such-config")).is_empty());
        let path = fixture("[services][dev.scene.Scene.desktop]\n_launch=none\n");
        assert!(read_shortcuts(&path).is_empty());
        fs::remove_file(path).expect("remove shortcut fixture");
    }

    #[test]
    fn session_detection_is_capability_shaped() {
        let kde = Inputs {
            desktop: "KDE".into(),
            session_type: "wayland".into(),
            config_path: PathBuf::from("/scene/no-such-config"),
        }
        .detect();
        assert!(matches!(kde.desktop, DesktopSupport::Kde { .. }));
        assert_eq!(
            kde.shortcut_summary(),
            "No active shortcut observed; packaged fallback: Meta+Space"
        );

        let other = Inputs {
            desktop: "GNOME".into(),
            session_type: "wayland".into(),
            config_path: PathBuf::from("/scene/no-such-config"),
        }
        .detect();
        assert!(matches!(other.desktop, DesktopSupport::Unsupported { .. }));
        assert!(other.recorder.is_none());
    }

    #[test]
    fn copilot_detection_requires_an_observed_recognisable_event() {
        assert_eq!(
            classify_copilot_key("F23", true, true),
            Some(CopilotStatus::BindableObserved)
        );
        assert_eq!(
            classify_copilot_key("XF86Assistant", false, false),
            Some(CopilotStatus::UnbindableObserved)
        );
        assert_eq!(classify_copilot_key("F23", false, true), None);
        assert_eq!(classify_copilot_key("F22", true, true), None);
    }
}
