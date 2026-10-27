//! Finding agent sessions inside tmux panes and working out what they are doing.
//!
//! Two signals are combined:
//!  1. The process tree under each pane's shell (is an agent running here?).
//!  2. The agent's own session record, then the pane title (is it working,
//!     asking for permission, idle?). The screen is only used for the preview.
//!
//! Optionally, Claude Code hooks (`agentop hook`) record the last lifecycle
//! event per pane, which lets us tell "finished a turn" apart from "idle" and
//! show how long ago it happened. See `state.rs`.

use crate::agents::{AGENTS, Agent, ContextUsage, ProcessInfo};
use crate::state::{self, HookState};
use crate::tmux::{self, Pane};
use anyhow::Result;
use serde::Serialize;
use std::collections::HashMap;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

pub use crate::agents::Status;

#[derive(Debug, Clone, Serialize)]
pub struct AgentSession {
    pub kind: &'static str,
    pub pane_id: String,
    pub session: String,
    pub window_index: u32,
    pub window_name: String,
    pub pane_index: u32,
    pub cwd: String,
    pub session_attached: bool,
    pub status: Status,
    /// One line of context: the question being asked, the spinner text, or
    /// the last notification.
    pub detail: String,
    /// Seconds since the last hook event, if hooks are installed.
    pub since_secs: Option<u64>,
    /// `None` when the agent does not expose it or the transcript is missing.
    pub context: Option<ContextUsage>,
    #[serde(skip)]
    pub screen: Vec<String>,
}

impl AgentSession {
    pub fn target(&self) -> String {
        format!("{}:{}.{}", self.session, self.window_index, self.pane_index)
    }
}

/// pid -> (ppid, args)
fn process_table() -> HashMap<u32, (u32, String)> {
    let out = Command::new("ps")
        .args(["-A", "-o", "pid=", "-o", "ppid=", "-o", "args="])
        .output();
    let mut map = HashMap::new();
    let Ok(out) = out else { return map };
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        // "  123   45 /usr/bin/claude --flag" — columns are space-padded.
        let (pid, rest) = split_field(line);
        let (ppid, args) = split_field(rest);
        if let (Ok(pid), Ok(ppid)) = (pid.parse(), ppid.parse()) {
            map.insert(pid, (ppid, args.trim().to_string()));
        }
    }
    map
}

fn split_field(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    s.split_once(char::is_whitespace).unwrap_or((s, ""))
}

/// Which agent (if any) runs in the process tree rooted at `root`, and the
/// pid of the process that matched.
fn agent_under(
    root: u32,
    procs: &HashMap<u32, (u32, String)>,
    children: &HashMap<u32, Vec<u32>>,
) -> Option<(&'static dyn Agent, u32)> {
    let mut stack = vec![root];
    while let Some(pid) = stack.pop() {
        if let Some((_, args)) = procs.get(&pid)
            && let Some(agent) = AGENTS.iter().find(|a| a.matches(args))
        {
            return Some((*agent, pid));
        }
        if let Some(kids) = children.get(&pid) {
            stack.extend(kids);
        }
    }
    None
}

fn shorten(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn merge_hook(
    status: Status,
    detail: String,
    hook: Option<&HookState>,
) -> (Status, String, Option<u64>) {
    let Some(h) = hook else {
        return (status, detail, None);
    };
    let since = Some(now().saturating_sub(h.ts));
    match (status, h.event.as_str()) {
        // Live states come from the agent itself. Unknown stays unknown.
        (Status::NeedsInput | Status::Working | Status::Unknown, _) => (status, detail, since),
        (_, "Stop") => (Status::Done, h.message.clone().unwrap_or_default(), since),
        (_, "Notification") => {
            let msg = h.message.clone().unwrap_or_default();
            let st = if msg.to_lowercase().contains("permission") {
                Status::NeedsInput
            } else {
                Status::Idle
            };
            (st, msg, since)
        }
        _ => (status, detail, since),
    }
}

pub fn home_short(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() && path.starts_with(&h) => format!("~{}", &path[h.len()..]),
        _ => path.to_string(),
    }
}

/// Scan every pane on the tmux server and return the ones running an agent.
pub fn scan() -> Result<Vec<AgentSession>> {
    let panes: Vec<Pane> = tmux::list_panes()?;
    if panes.is_empty() {
        return Ok(vec![]);
    }
    let procs = process_table();
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for (&pid, (ppid, _)) in &procs {
        children.entry(*ppid).or_default().push(pid);
    }
    state::prune(&panes.iter().map(|p| p.id.clone()).collect::<Vec<_>>());
    let hooks = state::load_all();

    let mut agents = Vec::new();
    for p in panes {
        let found = agent_under(p.pid, &procs, &children).or_else(|| {
            // Fallback when `ps` is unavailable/restricted.
            AGENTS
                .iter()
                .find(|a| a.matches(&p.current_command))
                .map(|a| (*a, p.pid))
        });
        let Some((agent, agent_pid)) = found else {
            continue;
        };

        let screen = tmux::capture(&p.id).unwrap_or_default();
        let proc = ProcessInfo {
            pid: agent_pid,
            cwd: p.current_path.clone(),
        };
        let (status, detail) = agent.classify(&proc, &p.title);
        let context = agent.context_usage(&proc);
        let (status, detail, since_secs) = merge_hook(status, detail, hooks.get(&p.id));

        agents.push(AgentSession {
            kind: agent.name(),
            pane_id: p.id,
            session: p.session,
            window_index: p.window_index,
            window_name: p.window_name,
            pane_index: p.pane_index,
            cwd: home_short(&p.current_path),
            session_attached: p.session_attached,
            status,
            detail: shorten(&detail, 120),
            since_secs,
            context,
            screen,
        });
    }

    // Most urgent first, then stable by location.
    agents.sort_by(|a, b| {
        a.status
            .cmp(&b.status)
            .then_with(|| a.session.cmp(&b.session))
            .then(a.window_index.cmp(&b.window_index))
            .then(a.pane_index.cmp(&b.pane_index))
    });
    Ok(agents)
}

pub fn fmt_age(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86399 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86400),
    }
}

pub fn context_text(a: &AgentSession) -> String {
    a.context
        .map(|c| fmt_tokens(c.used))
        .unwrap_or_else(|| "-".into())
}

pub fn fmt_tokens(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{}k", (n + 500) / 1000),
        _ => format!("{:.1}m", n as f64 / 1_000_000.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_tokens() {
        assert_eq!(fmt_tokens(950), "950");
        assert_eq!(fmt_tokens(84_400), "84k");
        assert_eq!(fmt_tokens(200_000), "200k");
        assert_eq!(fmt_tokens(1_000_000), "1.0m");
    }
}
