# 09 — Agents

`src/agents/` houses `MergeAgent` impls + tmux + prompts.

## tmux

Direct port of `agent-orchestrator`'s tmux module. One session per
MergeSmith run (`mergesmith-<short-pid>`) with one window per conflict
session. Detach-safe: closing the user's terminal doesn't kill the agent.

## Backends

| Backend | Binary | Launch shape |
|---|---|---|
| Opencode (default) | `opencode` | `opencode --resume-session …` inside the worktree |
| Claude Code | `claude` | `claude` inside the worktree with prompt piped via heredoc |
| Cursor | `cursor-agent` | `cursor-agent` inside the worktree (falls back to `open -a Cursor <path>`) |
| Codex | `codex` | `codex` inside the worktree with prompt argument |

All four wrap the user's CLI; MergeSmith never embeds a model itself.

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
         └─ user runs `mergesmith retry <id>` from the worktree
            └─ engine sets status = Queued, last_outcome = AgentResolvedConflict
               └─ on next claim, FSM routes to Rebase
                  └─ close_conflict_session(Resolved)
```

If MergeSmith dies mid-session, on restart the recovery sweep checks
whether the tmux session still exists. If gone, `close_conflict_session
(Abandoned)`. If alive, leave it; the user can attach with `tmux attach
-t mergesmith-…`.
