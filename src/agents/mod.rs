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
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::NeedsInput => "NEEDS INPUT",
            Status::Working => "WORKING",
            Status::Done => "DONE",
            Status::Idle => "IDLE",
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

    /// Screen fragments meaning the agent is waiting on the user.
    fn needs_input(&self) -> &'static [&'static str];

    /// Screen fragments (lowercase) meaning the agent is busy.
    fn working(&self) -> &'static [&'static str];

    /// Status and one line of detail from the visible screen.
    fn classify(&self, screen: &[String]) -> (Status, String) {
        classify_by_patterns(self.needs_input(), self.working(), screen)
    }

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

/// Clean the box-drawing/indicator decoration off a screen line.
fn tidy(line: &str) -> String {
    line.trim_matches(|c: char| c.is_whitespace() || "│╭╮╰╯─>❯·".contains(c))
        .to_string()
}

fn classify_by_patterns(
    needs_input: &[&str],
    working: &[&str],
    screen: &[String],
) -> (Status, String) {
    // Only the bottom of the screen reflects current state; older output
    // further up can contain stale prompts.
    let tail_start = screen.len().saturating_sub(30);
    let tail = &screen[tail_start..];

    for line in tail.iter().rev() {
        if needs_input.iter().any(|p| line.contains(p)) {
            // Prefer the question itself over the option list.
            let q = tail
                .iter()
                .rev()
                .find(|l| {
                    l.contains("Do you want")
                        || l.contains("Would you like")
                        || l.trim_end().ends_with('?')
                })
                .unwrap_or(line);
            return (Status::NeedsInput, tidy(q));
        }
    }
    let lower: Vec<String> = tail.iter().map(|l| l.to_lowercase()).collect();
    for (i, line) in lower.iter().enumerate().rev() {
        if working.iter().any(|p| line.contains(p)) {
            return (Status::Working, tidy(&tail[i]));
        }
    }
    (Status::Idle, String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<String> {
        s.lines().map(String::from).collect()
    }

    #[test]
    fn classifies_permission_prompt() {
        let s = lines(
            "● Bash(rm -rf build)\n╭──────╮\n│ Bash command\n│ Do you want to proceed?\n│ ❯ 1. Yes\n│   2. No\n╰──────╯",
        );
        let (st, d) = claude::Claude.classify(&s);
        assert_eq!(st, Status::NeedsInput);
        assert_eq!(d, "Do you want to proceed?");
    }

    #[test]
    fn classifies_working() {
        let s = lines("✻ Thinking… (12s · ↑ 1.2k tokens · esc to interrupt)\n\n> \n");
        let (st, d) = claude::Claude.classify(&s);
        assert_eq!(st, Status::Working);
        assert!(d.starts_with("✻ Thinking"));
    }

    #[test]
    fn classifies_idle() {
        let s = lines("● Done. All tests pass.\n╭────╮\n│ >  │\n╰────╯\n  ? for shortcuts");
        assert_eq!(claude::Claude.classify(&s).0, Status::Idle);
    }
}
