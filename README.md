# MergeQueue

A local merge queue with an animated Penrose-tiling visualizer. Register your
repos, enqueue worktrees from parallel agents, and let the queue merge them
into your default branches one by one — with CI gates and conflict-resolution
agents.

MergeQueue is a single Rust binary. It runs as a foreground TUI process when
you want to watch the queue work, and as one-shot CLI subcommands for
everything else.

## Quick start

```sh
brew install mergequeue/tap/mergequeue
cd ~/code/my-repo
mergequeue init
mergequeue              # opens the animated TUI; worker starts
# …in another terminal, inside any registered worktree:
mergequeue enqueue
```

## Requirements

- macOS or Linux (x86_64 or aarch64)
- Any terminal runs the TUI. The Penrose animation renders as a
  Braille-glyph wireframe, so it works everywhere; plain-text CLI
  subcommands work everywhere too.
- `git` ≥ 2.30 on PATH
- `tmux` on PATH (used for conflict-resolution agent sessions)
- `opencode` on PATH (for `mergequeue resolve`)

See [`docs/`](docs/) for the full plan.

## License

MIT OR Apache-2.0
