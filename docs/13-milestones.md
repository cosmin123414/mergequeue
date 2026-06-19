# 13 — Milestones

| M | Crates/modules touched | Deliverable |
|---|---|---|
| M1 | `core`, `store`, `git`, `engine`, `cli` (no `resolve`, no `tui`) | Daemon-free FIFO merge queue. `init`, `enqueue`, `status`, `cancel`, `retry`, `logs`, `repos`. Rebase-then-FF + CI gate. **Dogfoodable from a terminal.** |
| M2 | `agents`, `cli::commands::resolve` | Opencode backend; `mergequeue resolve` opens tmux agent session. `NeedsHelp` end-to-end. |
| M3 | `engine::pool`, `store` | Multiple registered repos; per-repo workers running in parallel; dirty-target retry. |
| M4 | `tui` (whole module), `cli::commands::tui` | Animated TUI with the Penrose-tiling visualizer, rendered as a Braille-glyph wireframe (works in any terminal). Three states (idle/working/needs-help). |
| M5 | `tui::sprite::penrose` | Tuned palette + per-state zoom. |
| ~~M6~~ | ~~`agents::{claude_code,cursor,codex}`~~ | **Dropped.** MergeQueue ships opencode only; the `AgentRegistry` seam remains for future backends. |
| M7 | `justfile`, `.github/workflows/`, README, asciinema casts | Homebrew tap + GH Releases + screenshots. Public launch. |

Each milestone ships a working, testable artifact. M1 ships before any
pixel is drawn.
