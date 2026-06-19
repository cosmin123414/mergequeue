# 00 — Overview

## What MergeQueue is

A developer tool that consumes finished work from parallel agents (or your own
worktrees) and merges it into a default branch, one entry at a time per repo.
Each entry goes through a precheck, a rebase-onto-target, a CI gate
(`lint → test → build`), and a fast-forward. Conflicts spawn an agent in a
tmux window so you can fix them and re-enqueue.

## What MergeQueue is not

- Not a daemon. Merges only happen while `mergequeue` (the TUI) is running.
  Enqueued entries persist; they're processed when you next open the TUI.
- Not a CI service. It runs your locally-defined CI commands; it doesn't
  schedule, parallelize, or distribute them.
- Not a worktree creator. It assumes you (or your agents) already create
  worktrees. MergeQueue only consumes finished work.
- Not a Mac App Store app. It runs in terminals on macOS and Linux.

## The visualizer

An animated Penrose tiling (an infinite emergence-zoom over a P3 rhombus
tiling) is rendered into the TUI as a Braille-glyph wireframe, so it
works in any terminal. The animation's mood is tied to engine activity:

- **Idle** — queue is empty (slow zoom, muted palette)
- **Working** — an entry is rebasing, running CI, or fast-forwarding
  (faster zoom, livelier accent)
- **NeedsHelp** — an entry is blocked on a conflict (amber warning accent)

The animation is the headline UX. The rest of the TUI is a queue table + log
tail.
