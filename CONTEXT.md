# agentop

A terminal dashboard that lists coding agents running in tmux panes and shows what each one is doing.

## Language

**Agent**:
A kind of coding tool, such as Claude Code, Codex or opencode. Each agent knows how to recognize its own process, report its own status, and report its own context usage.
_Avoid_: tool, provider, def

**Agent session**:
One running instance of an agent inside one tmux pane. This is what a table row represents.
_Avoid_: agent (for an instance), pane, process

**Title**:
The label of an agent session. The name the user set with `/rename` if there is one, otherwise the title Claude Code generates from the conversation (`ai-title` in the transcript), otherwise a generated name. Shown in the TITLE column. Not the tmux session, which has its own TMUX column.
_Avoid_: session name, session (alone)

**Status**:
What an agent session is doing right now: needs input, working, done, idle, or unknown when no reliable signal exists.

**Filter**:
A fuzzy query the user types after pressing `/` that narrows the table to the agent sessions whose status, tmux session, title, directory or detail match it. Clearing it restores every row.
_Avoid_: search, query (alone)

**Context usage**:
The number of tokens an agent session currently holds in its context, as recorded in the agent's own transcript.
_Avoid_: memory, tokens (alone)

**Origin**:
The agent session whose pane the user was in when they opened the dashboard. Not every dashboard opening has one, since the user may start from a pane that runs no agent.
_Avoid_: current (tmux already uses "current" for the active pane)

**Last used**:
The agent session the user focused most recently before the origin, or the most recent one overall when there is no origin. Focus means selecting its pane, window or session, not the agent doing work.
_Avoid_: previous, recent
