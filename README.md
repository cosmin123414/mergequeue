<h1 align="center">mergequeue</h1>

<p align="center">
  A local, daemon-free merge queue. Enqueue finished worktrees, let it
  rebase each onto your default branch behind a CI gate, and fast-forward
  them in one at a time — with an animated Penrose-tiling visualizer.
</p>

<p align="center">
  <a href="#license"><img alt="License" src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue?style=flat-square"></a>
  <img alt="Rust" src="https://img.shields.io/badge/rust-2021-orange?style=flat-square">
</p>

https://github.com/user-attachments/assets/ecce6fbb-f65b-4f8a-9c6c-0c5aeed5e1ea

---

## What it is

mergequeue is a single Rust binary that merges finished work into a
default branch, one entry at a time per repo. Each entry runs through:

1. **Precheck** the target worktree is clean.
2. **Rebase** the source branch onto the target.
3. **CI gate** — your local `lint → test → build` commands.
4. **Fast-forward** the target onto the rebased branch.

A conflict opens an [opencode](https://opencode.ai) agent in a tmux
window so you can resolve it and re-enqueue. State is global, so one
dashboard tracks entries across many repos.

It is **not a daemon**: merges only happen while the TUI is running.
Enqueued entries persist in SQLite and are processed the next time you
open it.

## Requirements

- Rust 1.89+ (Edition 2021)
- macOS or Linux (x86_64 or aarch64)
- `git` ≥ 2.30, `tmux`
- [`opencode`](https://opencode.ai) on `PATH` (for conflict resolution)

Any terminal works — the visualizer renders as a Braille-glyph
wireframe, no special graphics protocol required.

## Install

```sh
cargo install --path .
```

> [!NOTE]
> Pre-1.0, install from source. No crates.io release yet.

## Quick start

```sh
cd ~/code/my-repo
mergequeue init          # register this repo
mergequeue               # open the TUI; the worker starts

# …in another terminal, inside a registered worktree:
mergequeue enqueue
```

Keybinds (in the TUI):

| Key | Action |
| --- | --- |
| `j` / `k` or `↑` / `↓` | Move selection in the queue table. |
| `d` | Delete the focused entry (only if `Queued`). |
| `Enter` / `r` | Attach to the focused `NeedsHelp` entry's agent session. |
| `?` | Toggle the help overlay. |
| `q` | Quit (soft shutdown). |
| `Q` | Force quit (hard shutdown). |

Every other mutation — enqueue, cancel, retry, resolve — is a CLI
subcommand, so the TUI stays a read-mostly visualizer.

## How it works

### 1. Register a repo and enqueue work

`mergequeue init` registers the current repo. From any worktree of a
registered repo (typically a `git worktree add` sibling created by you or
your agents), `mergequeue enqueue` records that worktree's branch as a
queue entry targeting the default branch. mergequeue never creates
worktrees itself — it only consumes finished work.

### 2. The queue does the merge

One worker per registered repo claims entries FIFO and drives each
through precheck → rebase → CI → fast-forward. Workers are isolated: a
slow CI on repo A never blocks repo B. The animated Penrose tiling
reflects engine activity — idle, working, or needs-help.

### 3. Conflicts hand off to an agent

When a rebase conflicts, the entry becomes `NeedsHelp` and an opencode
session opens in a tmux window. Resolve the conflict there, then
`mergequeue retry <id>` to re-enqueue. If mergequeue dies mid-session,
the next startup sweeps the abandoned session and reclaims orphaned
entries.

## CLI

| Command | Effect |
| --- | --- |
| `mergequeue` / `mergequeue tui` | Open the animated TUI dashboard. |
| `mergequeue init` | Register the current repo. |
| `mergequeue enqueue [--target <branch>] [--message <msg>]` | Queue the current worktree for merge. |
| `mergequeue status [--format json]` | Print the live queue. |
| `mergequeue cancel <id>` | Cancel a queued or in-flight entry. |
| `mergequeue retry <id>` | Re-enqueue a `Failed` or `NeedsHelp` entry. |
| `mergequeue resolve <id>` | Attach to a `NeedsHelp` entry's agent session. |
| `mergequeue logs <id>` | Print CI + merge logs for an entry. |
| `mergequeue repos {list,remove}` | Manage registered repos. |
| `mergequeue doctor` | Environment sanity check. |

## Configuration

<details>
<summary><strong>State root and logs</strong></summary>

**State root** (sqlite + per-entry run artifacts) resolves in this order:

1. `$MERGEQUEUE_HOME`
2. macOS: `~/Library/Application Support/MergeQueue`
3. Linux: `$XDG_DATA_HOME/mergequeue` (default `~/.local/share/mergequeue`)

Layout:

```
$MERGEQUEUE_HOME/
  state.sqlite[+ wal + shm]   queue state; the only coordination mechanism
  tui.pid                     singleton lock for `mergequeue tui`
  runs/<entry-id>/            ci-{lint,test,build}.log, rebase.log, merge.log
  config.toml
```

**Logging**: set `MERGEQUEUE_LOG` (an `env_filter` directive, e.g.
`MERGEQUEUE_LOG=debug`).

</details>

## Status

Pre-1.0. APIs, schemas, and on-disk layouts may change without notice.

## Contributing

The repo keeps an "Intent Layer" of `CLAUDE.md` files alongside the code
describing each module's contract — start at the root
[`CLAUDE.md`](CLAUDE.md), and see [`docs/`](docs/) for the full design.

## License

MIT OR Apache-2.0 — see [LICENSE](LICENSE).
