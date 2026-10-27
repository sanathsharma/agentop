//! Per-pane state written by Claude Code hooks (`agentop hook`).
//!
//! Claude Code runs hooks with the agent's environment, so `$TMUX_PANE`
//! tells us which pane the event belongs to. One small JSON file per pane:
//!   $XDG_STATE_HOME/agentop/panes/<pane-number>.json

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookState {
    pub event: String,
    pub ts: u64,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
}

fn dir() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state")
        });
    base.join("agentop").join("panes")
}

fn file_for(pane_id: &str) -> PathBuf {
    dir().join(format!("{}.json", pane_id.trim_start_matches('%')))
}

pub fn load_all() -> HashMap<String, HookState> {
    let mut map = HashMap::new();
    let Ok(entries) = std::fs::read_dir(dir()) else {
        return map;
    };
    for e in entries.flatten() {
        let path = e.path();
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if let Ok(s) = std::fs::read_to_string(&path)
            && let Ok(st) = serde_json::from_str::<HookState>(&s)
        {
            map.insert(format!("%{stem}"), st);
        }
    }
    map
}

/// Entry point for `agentop hook`: reads the hook JSON from stdin and records it.
/// Never fails loudly — a broken hook must not disturb the agent.
pub fn record_from_stdin() -> Result<()> {
    let Some(pane) = std::env::var("TMUX_PANE").ok().filter(|p| !p.is_empty()) else {
        return Ok(()); // not running inside tmux; nothing to track
    };
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).ok();
    let v: serde_json::Value = serde_json::from_str(&input).unwrap_or_default();

    let event = v["hook_event_name"].as_str().unwrap_or("").to_string();
    let path = file_for(&pane);

    if event == "SessionEnd" {
        let _ = std::fs::remove_file(path);
        return Ok(());
    }

    let message = match event.as_str() {
        "Notification" => v["message"].as_str().map(String::from),
        "UserPromptSubmit" => v["prompt"]
            .as_str()
            .map(|p| format!("› {}", p.lines().next().unwrap_or(""))),
        _ => None,
    };
    let st = HookState {
        event,
        ts: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        message,
        session_id: v["session_id"].as_str().map(String::from),
    };
    std::fs::create_dir_all(dir()).context("creating state dir")?;
    // Write-then-rename so the TUI never reads a half-written file.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(&st)?)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

/// Remove state files for panes that no longer exist.
pub fn prune(live_panes: &[String]) {
    let Ok(entries) = std::fs::read_dir(dir()) else {
        return;
    };
    for e in entries.flatten() {
        let path = e.path();
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            && !live_panes.iter().any(|p| p.trim_start_matches('%') == stem)
        {
            let _ = std::fs::remove_file(path);
        }
    }
}
