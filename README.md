# MergeSmith

A local merge queue with a pixel-art blacksmith. Register your repos, enqueue
worktrees from parallel agents, and let the smith merge them into your default
branches one by one — with CI gates and conflict-resolution agents.

MergeSmith is a single Rust binary. It runs as a foreground TUI process when
you want to watch the smith hammer, and as one-shot CLI subcommands for
everything else.

## Quick start

```sh
brew install mergesmith/tap/mergesmith
cd ~/code/my-repo
mergesmith init
mergesmith              # opens the animated TUI; worker starts
# …in another terminal, inside any registered worktree:
mergesmith enqueue
```

## Requirements

- macOS or Linux (x86_64 or aarch64)
- A terminal that speaks the Kitty graphics protocol for `mergesmith tui`:
  Kitty, Ghostty, WezTerm, or iTerm2 (≥ 3.5). Plain-text CLI subcommands work
  in any terminal.
- `git` ≥ 2.30 on PATH
- `tmux` on PATH (used for conflict-resolution agent sessions)
- One of: `opencode`, `claude`, `cursor-agent`, `codex` (for `mergesmith resolve`)

See [`docs/`](docs/) for the full plan.

## License

MIT OR Apache-2.0
