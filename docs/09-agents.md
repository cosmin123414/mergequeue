# 09 — Agents

`src/agents/` houses `MergeAgent` impls + tmux + prompts.

## tmux

Direct port of `agent-orchestrator`'s tmux module. One session per
MergeQueue run (`mergequeue-<short-pid>`) with one window per conflict
session. Detach-safe: closing the user's terminal doesn't kill the agent.

## Backends

| Backend | Binary | Launch shape |
|---|---|---|
| Opencode | `opencode` | `opencode` inside the worktree, prompt sent via `tmux send-keys` |

MergeQueue ships a single backend (opencode); it wraps the user's CLI
and never embeds a model itself. The `MergeAgent` trait and the
`AgentRegistry` seam are kept so additional backends can be added later
without touching the engine.

## Prompts

Two minijinja templates in `src/agents/prompts/`:

- `conflict_system.md` — invariants the agent must respect (don't push,
  don't change branch, resolve conflicts only, run CI after resolution).
- `conflict_user.md.j2` — rendered with `{ repo, source_branch,
  target_branch, conflicted_files, ci_command }`.

Rendered into a single message handed to the agent via its CLI's
prompt-injection mechanism (varies per backend).

## ConflictSession lifecycle

```
open_conflict_session
   └─ records started_at, tmux_session/window in DB
      └─ user works in the tmux window
         └─ user runs `mergequeue retry <id>` from the worktree
            └─ engine sets status = Queued, last_outcome = AgentResolvedConflict
               └─ on next claim, FSM routes to Rebase
                  └─ close_conflict_session(Resolved)
```

If MergeQueue dies mid-session, on restart the recovery sweep checks
whether the tmux session still exists. If gone, `close_conflict_session
(Abandoned)`. If alive, leave it; the user can attach with `tmux attach
-t mergequeue-…`.
