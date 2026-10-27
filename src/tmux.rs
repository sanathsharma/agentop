//! Thin wrapper around the `tmux` binary. Everything goes through the tmux
//! server, so panes from every session (attached or not) are visible.

use anyhow::{Context, Result, bail};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct Pane {
    /// Stable pane id, e.g. `%12`. Used as the target for every tmux command.
    pub id: String,
    pub session: String,
    pub window_index: u32,
    pub window_name: String,
    pub pane_index: u32,
    pub pid: u32,
    pub current_command: String,
    pub current_path: String,
    pub title: String,
    pub session_attached: bool,
}

/// tmux escapes control characters (and tabs) in -F output, so use a
/// printable sequence that won't occur in session names or paths.
const SEP: &str = "<|>";

pub fn tmux(args: &[&str]) -> Result<String> {
    let out = Command::new("tmux")
        .args(args)
        .output()
        .context("failed to run `tmux` (is it installed and on PATH?)")?;
    if !out.status.success() {
        bail!(
            "tmux {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// All panes across all sessions on the current tmux server.
pub fn list_panes() -> Result<Vec<Pane>> {
    let fields = [
        "#{pane_id}",
        "#{session_name}",
        "#{window_index}",
        "#{window_name}",
        "#{pane_index}",
        "#{pane_pid}",
        "#{pane_current_command}",
        "#{pane_current_path}",
        "#{pane_title}",
        "#{session_attached}",
    ];
    let fmt = fields.join(SEP);
    let raw = match tmux(&["list-panes", "-a", "-F", &fmt]) {
        Ok(s) => s,
        // No server running is not an error for us: just no agents.
        Err(e) if e.to_string().contains("no server running") => return Ok(vec![]),
        Err(e) => return Err(e),
    };

    let mut panes = Vec::new();
    for line in raw.lines() {
        let f: Vec<&str> = line.split(SEP).collect();
        if f.len() != fields.len() {
            continue;
        }
        panes.push(Pane {
            id: f[0].to_string(),
            session: f[1].to_string(),
            window_index: f[2].parse().unwrap_or(0),
            window_name: f[3].to_string(),
            pane_index: f[4].parse().unwrap_or(0),
            pid: f[5].parse().unwrap_or(0),
            current_command: f[6].to_string(),
            current_path: f[7].to_string(),
            title: f[8].to_string(),
            session_attached: f[9] != "0",
        });
    }
    Ok(panes)
}

/// Visible contents of a pane as plain text (no escape sequences),
/// with wrapped lines joined and trailing blank lines removed.
pub fn capture(pane_id: &str) -> Result<Vec<String>> {
    let raw = tmux(&["capture-pane", "-p", "-J", "-t", pane_id])?;
    let mut lines: Vec<String> = raw.lines().map(|l| l.trim_end().to_string()).collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    Ok(lines)
}

pub fn inside_tmux() -> bool {
    std::env::var_os("TMUX").is_some()
}

/// Make `pane_id` the active pane of its window and that window the current
/// window of its session. Works from anywhere.
pub fn focus_pane(pane_id: &str) -> Result<()> {
    tmux(&["select-window", "-t", pane_id])?;
    tmux(&["select-pane", "-t", pane_id])?;
    Ok(())
}

/// From inside tmux (including a `display-popup`): move this client to the
/// session that owns `pane_id` and focus it.
pub fn switch_to(pane_id: &str) -> Result<()> {
    focus_pane(pane_id)?;
    tmux(&["switch-client", "-t", pane_id])?;
    Ok(())
}

/// From outside tmux: replace this process with `tmux attach` on the target.
/// Only returns on failure.
pub fn exec_attach(pane_id: &str, session: &str) -> anyhow::Error {
    use std::os::unix::process::CommandExt;
    if let Err(e) = focus_pane(pane_id) {
        return e;
    }
    let err = Command::new("tmux")
        .args(["attach-session", "-t", session])
        .exec();
    anyhow::Error::new(err).context("failed to exec `tmux attach-session`")
}
