//! Desktop notifications and sounds for `agentop hook --notify`.
//!
//! Linux uses `notify-send`. macOS uses `terminal-notifier` when installed
//! and `osascript` otherwise. Every call is best effort: a missing tool or
//! sound file is skipped, and nothing here may block or fail the hook.

use crate::tmux;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Off,
    /// Only when the agent needs input.
    Input,
    /// Needs input and finished turns.
    All,
}

impl Mode {
    pub fn parse(s: &str) -> Mode {
        match s {
            "input" => Mode::Input,
            "all" => Mode::All,
            _ => Mode::Off,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    NeedsInput,
    Done,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::NeedsInput => "Needs input",
            Kind::Done => "Done",
        }
    }

    fn sound_env(self) -> &'static str {
        match self {
            Kind::NeedsInput => "AGENTOP_SOUND_INPUT",
            Kind::Done => "AGENTOP_SOUND_DONE",
        }
    }

    fn default_sound(self) -> &'static str {
        match (cfg!(target_os = "macos"), self) {
            (true, Kind::NeedsInput) => "/System/Library/Sounds/Glass.aiff",
            (true, Kind::Done) => "/System/Library/Sounds/Hero.aiff",
            (false, Kind::NeedsInput) => {
                "/usr/share/sounds/freedesktop/stereo/message-new-instant.oga"
            }
            (false, Kind::Done) => "/usr/share/sounds/freedesktop/stereo/complete.oga",
        }
    }
}

/// Which notification, if any, a hook event produces under `mode`.
pub fn classify(mode: Mode, event: &str) -> Option<Kind> {
    match (mode, event) {
        (Mode::Off, _) => None,
        (_, "Notification") => Some(Kind::NeedsInput),
        (Mode::All, "Stop") => Some(Kind::Done),
        _ => None,
    }
}

pub struct PaneInfo {
    pub session: String,
    /// The pane is the active pane of an attached tmux client, so the
    /// user is already looking at it.
    pub in_view: bool,
}

pub fn pane_info(pane: &str) -> Option<PaneInfo> {
    let out = tmux::tmux(&[
        "display-message",
        "-p",
        "-t",
        pane,
        "#{pane_active} #{window_active} #{session_attached} #{session_name}",
    ])
    .ok()?;
    let mut f = out.trim_end().splitn(4, ' ');
    let (active, window, attached, session) = (f.next()?, f.next()?, f.next()?, f.next()?);
    Some(PaneInfo {
        session: session.to_string(),
        in_view: active == "1" && window == "1" && attached != "0",
    })
}

/// The title names the agent and its tmux session. The body is the agent's
/// own message, or the kind of event when there is none.
pub fn notify(kind: Kind, agent: &str, session: &str, message: &str, sound: bool) {
    let title = format!("{agent} \u{b7} {session}");
    let body = if message.is_empty() {
        kind.label()
    } else {
        message
    };
    send(&title, body);
    if sound {
        play(kind);
    }
}

fn send(title: &str, body: &str) {
    if cfg!(target_os = "macos") {
        if on_path("terminal-notifier") {
            spawn("terminal-notifier", &["-title", title, "-message", body]);
        } else {
            // Text goes in as argv so quotes in it cannot break the script.
            spawn(
                "osascript",
                &[
                    "-e",
                    "on run argv",
                    "-e",
                    "display notification (item 1 of argv) with title (item 2 of argv)",
                    "-e",
                    "end run",
                    body,
                    title,
                ],
            );
        }
    } else {
        spawn("notify-send", &["--app-name=agentop", title, body]);
    }
}

fn play(kind: Kind) {
    let file = std::env::var_os(kind.sound_env())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(kind.default_sound()));
    if !file.is_file() {
        return;
    }
    let player = if cfg!(target_os = "macos") {
        &["afplay"][..]
    } else {
        &["pw-play", "paplay", "canberra-gtk-play"][..]
    };
    let Some(bin) = player.iter().find(|p| on_path(p)) else {
        return;
    };
    let path = file.to_string_lossy();
    if *bin == "canberra-gtk-play" {
        spawn(bin, &["-f", &path]);
    } else {
        spawn(bin, &[&path]);
    }
}

/// Start `bin` and leave it running. The hook exits right away.
fn spawn(bin: &str, args: &[&str]) {
    let _ = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| is_executable(&d.join(bin))))
        .unwrap_or(false)
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_events_by_mode() {
        assert_eq!(classify(Mode::Off, "Notification"), None);
        assert_eq!(
            classify(Mode::Input, "Notification"),
            Some(Kind::NeedsInput)
        );
        assert_eq!(classify(Mode::Input, "Stop"), None);
        assert_eq!(classify(Mode::All, "Stop"), Some(Kind::Done));
        assert_eq!(classify(Mode::All, "UserPromptSubmit"), None);
    }

    #[test]
    fn parses_modes() {
        assert_eq!(Mode::parse("input"), Mode::Input);
        assert_eq!(Mode::parse("all"), Mode::All);
        assert_eq!(Mode::parse("bogus"), Mode::Off);
    }
}
