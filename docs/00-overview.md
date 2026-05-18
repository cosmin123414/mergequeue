# 00 — Overview

## What MergeSmith is

A developer tool that consumes finished work from parallel agents (or your own
worktrees) and merges it into a default branch, one entry at a time per repo.
Each entry goes through a precheck, a rebase-onto-target, a CI gate
(`lint → test → build`), and a fast-forward. Conflicts spawn an agent in a
tmux window so you can fix them and re-enqueue.

## What MergeSmith is not

- Not a daemon. Merges only happen while `mergesmith` (the TUI) is running.
  Enqueued entries persist; they're processed when you next open the TUI.
- Not a CI service. It runs your locally-defined CI commands; it doesn't
  schedule, parallelize, or distribute them.
- Not a worktree creator. It assumes you (or your agents) already create
  worktrees. MergeSmith only consumes finished work.
- Not a Mac App Store app. It runs in terminals on macOS and Linux.

## The blacksmith

A 64×64 pixel-art blacksmith is rendered into the TUI via the Kitty graphics
protocol. The smith has six animation states tied to engine activity:

- **Idle** — queue is empty
- **Working** — entry is rebasing or fast-forwarding
- **CI** — entry is running CI commands (bellows + glowing forge)
- **NeedsHelp** — an entry is blocked on a conflict
- **Celebrate** — fired once on `Merged`
- **Failed** — fired once on `Failed`

The sprite is the headline UX. The rest of the TUI is a queue table + log tail.
