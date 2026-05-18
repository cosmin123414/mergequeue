# 13 — Milestones

| M | Crates/modules touched | Deliverable |
|---|---|---|
| M1 | `core`, `store`, `git`, `engine`, `cli` (no `resolve`, no `tui`) | Daemon-free FIFO merge queue. `init`, `enqueue`, `status`, `cancel`, `retry`, `logs`, `repos`. Rebase-then-FF + CI gate. **Dogfoodable from a terminal.** |
| M2 | `agents`, `cli::commands::resolve` | Opencode backend; `mergesmith resolve` opens tmux agent session. `NeedsHelp` end-to-end. |
| M3 | `engine::pool`, `store` | Multiple registered repos; per-repo workers running in parallel; dirty-target retry. |
| M4 | `tui` (whole module), `cli::commands::tui` | Animated TUI with placeholder blacksmith sprite from itch.io. Three states (idle/working/needs-help). tmux passthrough validated. |
| M5 | `art/`, `assets/`, `tui::sprite` | Commissioned 6-state blacksmith. Celebrate one-shot. |
| M6 | `agents::{claude_code,cursor,codex}` | Three additional agent backends. Per-repo override. |
| M7 | `justfile`, `.github/workflows/`, README, asciinema casts | Homebrew tap + GH Releases + screenshots from Ghostty. Public launch. |

Each milestone ships a working, testable artifact. M1 ships before any
pixel is drawn.
