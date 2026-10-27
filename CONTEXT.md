# agentop

A terminal dashboard that lists coding agents running in tmux panes and shows what each one is doing.

## Language

**Agent**:
A kind of coding tool, such as Claude Code, Codex or opencode. Each agent knows how to recognize its own process, read its own screen, and report its own context usage.
_Avoid_: tool, provider, def

**Agent session**:
One running instance of an agent inside one tmux pane. This is what a table row represents.
_Avoid_: agent (for an instance), pane, process

**Status**:
What an agent session is doing right now: needs input, working, done, or idle.

**Context usage**:
The number of tokens an agent session currently holds in its context, as recorded in the agent's own transcript.
_Avoid_: memory, tokens (alone)
