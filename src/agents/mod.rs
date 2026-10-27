//! Supported coding agents.
//!
//! Each agent implements [`Agent`]. The rest of the program only talks to the
//! trait, so adding Codex or opencode means adding one file here and one line
//! to [`AGENTS`].

pub mod claude;

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Blocked on you: permission prompt, question, plan approval…
    NeedsInput,
    /// Actively generating / running tools.
    Working,
    /// Finished its last turn (known from the Stop hook).
    Done,
    /// Sitting at an empty prompt.
    Idle,
    /// No reliable signal. Shown instead of a guess.
    Unknown,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::NeedsInput => "NEEDS INPUT",
            Status::Working => "WORKING",
            Status::Done => "DONE",
            Status::Idle => "IDLE",
            Status::Unknown => "UNKNOWN",
        }
    }
}

/// How much context an agent session has used.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ContextUsage {
    pub used: u64,
}

/// The process an agent was recognized by.
pub struct ProcessInfo {
    pub pid: u32,
    /// Working directory of the pane, unshortened.
    pub cwd: String,
}

pub trait Agent: Sync {
    /// Short lowercase name shown in the AGENT column.
    fn name(&self) -> &'static str;

    /// Whether a process command line (argv joined by spaces) is this agent.
    fn matches(&self, cmdline: &str) -> bool;

    /// Status and one line of detail, from the agent's own session record
    /// or, failing that, the pane title. Returns `Unknown` when neither says.
    fn classify(&self, proc: &ProcessInfo, title: &str) -> (Status, String);

    /// Context usage of the session behind `proc`, if the agent exposes it.
    fn context_usage(&self, _proc: &ProcessInfo) -> Option<ContextUsage> {
        None
    }

    /// Setup instructions printed by `agentop init`.
    fn init_text(&self) -> Option<&'static str> {
        None
    }
}

pub static AGENTS: &[&dyn Agent] = &[&claude::Claude];
