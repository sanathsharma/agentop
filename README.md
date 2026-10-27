# agentop

A `top`-style dashboard for Claude Code agents running anywhere on your tmux
server. Every session is scanned (attached or not), each agent shows whether
it is **working**, **waiting on you**, **done**, **idle**, or **unknown**, and Enter takes
you straight to its pane — across sessions.

```
 agentop   3 agents   ● 1 needs input   ◐ 1 working   ✓ 1 done
╭ tmux agents ───────────────────────────────────────────────────────────────╮
│  #  STATUS        AGENT   SESSION  CONTEXT    AGE  DIRECTORY  DETAIL       │
│▶ 1  ● NEEDS INPUT claude  api      84k   -    ~/api      Do you want to proceed?
│  2  ◐ WORKING     claude  api      52k   -    ~/api      ✻ Compacting… (42s · esc to interrupt)
│  3  ✓ DONE        claude  web      12k   18s  ~/web                   │
╰────────────────────────────────────────────────────────────────────────────╯
╭ api:0.0  ~/api  (live preview of the selected pane) ───────────────────────╮
```

## Install

```sh
cargo install --path .        # Rust 1.88+ (edition 2024)
agentop init                  # prints the tmux + Claude Code snippets below
```

## Keys

| Key            | Action                                             |
|----------------|----------------------------------------------------|
| `↑↓` / `jk`    | Move                                               |
| `Enter` / `l`  | Switch to that pane (attaches if run outside tmux) |
| `1`–`9`        | Switch to row N directly                           |
| `Tab` / `n`    | Select the next agent that needs input             |
| `p`            | Toggle the pane preview                            |
| `r`            | Refresh now                                        |
| `q` / `Esc`    | Quit                                               |

## Commands

```
agentop               # TUI (refreshes every second)
agentop --stay        # don't exit after switching (for a dedicated dashboard window)
agentop -i 500        # refresh interval in ms
agentop list [--json] # one-shot output for scripts / status bars
agentop next          # jump to the first agent waiting for input
agentop hook          # Claude Code hook handler (see below)
agentop init          # print config snippets
```

## tmux setup

```tmux
bind "a" display-popup -E -w 90% -h 90% "agentop"    # prefix+a: dashboard popup
bind "A" run-shell "agentop next"                    # prefix+A: jump to next waiting agent
```

From the popup, Enter switches your client to the target session/window/pane
and closes the popup.

## How status is detected

1. **Which panes are agents.** `tmux list-panes -a` gives every pane's shell
   PID; the process tree under it is searched for `claude` (native binary or
   `node …/claude-code/cli.js`).
2. **What the agent is doing.** For Claude Code, the `status` field in
   `~/.claude/sessions/<pid>.json` (`busy`, `waiting`, `idle`) is used first.
   If it is missing, the pane title decides: a spinner glyph means
   **WORKING** and `✳` means **IDLE**. The screen is never parsed. If neither
   source says, the status is **UNKNOWN** (shown as `-`).
3. **Hooks (optional, recommended).** With the hooks from `agentop init`
   added to `~/.claude/settings.json`, Claude Code reports `Stop`,
   `Notification`, `UserPromptSubmit` and `SessionEnd` per pane (keyed by
   `$TMUX_PANE`) into `~/.local/state/agentop/panes/`. That upgrades IDLE to
   **DONE** after a turn finishes, fills the AGE column, and catches
   permission notifications even when the prompt isn't on screen. State files
   for closed panes are cleaned up automatically.

## Context usage

The CONTEXT column shows tokens used by the session, such as `99k`. It comes
from the agent's own session transcript (for Claude Code, the last `usage`
block under `~/.claude/projects/`). The window size is not recorded anywhere
agentop can read, so there is no percentage.

## Adding other agents

Each agent is a struct implementing the `Agent` trait in `src/agents/`
(see `claude.rs`): a process matcher, a `classify` method that returns the status, and optionally `context_usage` and `init_text`. Add the
file, then add one line to `AGENTS` in `src/agents/mod.rs`. Nothing else
changes.
