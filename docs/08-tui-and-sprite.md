# 08 — TUI and visualizer

## Layout

```
┌─ MergeQueue ────────────────────────────────────────────────────┐
│ ╔═══════════╗   Queue                                           │
│ ║ [penrose] ║   ┌──────────────────────────────────────────┐    │
│ ║  tiling   ║   │ ⚒ Rebasing  feat/auth → main  agent-orc │    │
│ ║ animation ║   │ ⏳ CI       feat/ui   → main  mergeq.   │    │
│ ║   here    ║   │ ⏳ Queued   chore     → main  mergeq.   │    │
│ ╚═══════════╝   │ ⚠ Help     feat/api  → dev   other     │    │
│                 │ ✓ Merged   fix/typo  → main  agent-orc │    │
│ Working…        └──────────────────────────────────────────┘    │
│ feat/auth → main · 00:42 elapsed                                │
│                                                                 │
│ ─ Log (active: feat/auth) ──────────────────────────────────────│
│   $ cargo test                                                  │
│   running 283 tests                                             │
│   test result: ok. 283 passed                                   │
│                                                                 │
│ [↑↓ select]  [d delete-queued]  [? help]  [q quit]              │
└─────────────────────────────────────────────────────────────────┘
```

## Keybinds

```
↑/↓ or j/k    Move selection in queue table
d             Delete selected entry (only if status = Queued)
?             Help overlay
q             Quit (soft shutdown)
Q             Force quit (hard shutdown)
```

Nothing else. Repo registration, enqueue, cancel-in-flight, retry, resolve
are all CLI subcommands.

## Visualizer rendering

A single renderer: a **Braille-glyph wireframe** that works in any
terminal. Each render tick the animator advances the tiling and
rasterizes its edges into a high-resolution dot bitmap (`cols*2 ×
rows*4`, since each Braille cell is a 2×4 dot matrix). `tui::sprite::glyph`
maps that bitmap to colored ratatui `Line`s of Braille glyphs, drawn into
the sprite rect by `tui::ui::draw`. There is no separate pixel path and
no terminal-capability probe.

### Visualizer states

The chosen state tunes the animation's zoom speed and accent color; the
underlying P3 tiling geometry is identical across states.

```
Idle       slow zoom, muted green accent          (queue empty)
Working    faster zoom, livelier green accent      (Rebasing / CI / Merging)
NeedsHelp  slowest zoom, amber warning accent      (any NeedsHelp present)
```

State priority when multiple entries are active across repos:
NeedsHelp > Working > Idle.

### Animation

The animator is a Rust port of the `penrose-loader` reference. It carries a
COARSE Robinson-triangle tiling plus its FINE and next-finer subdivisions;
per cycle the camera zooms in by the golden ratio φ while finer detail
fades in center-out via two overlapping reveal waves. At the wrap it
promotes `fine → coarse` (an exact identity) and rebuilds, so the infinite
zoom has no visible seam. See `src/tui/sprite/penrose.rs`.

### Frame timing

The render loop ticks at the animation frame rate; the animator is
advanced by the real inter-frame delta each iteration. Because the zoom
is time-based rather than frame-indexed, motion stays smooth regardless
of the actual frame rate, and a data refresh + pool reconcile is
throttled to ~5 Hz independently of the animation.

## Queue data

The render loop reads queue state directly from the SQLite store on each
throttled data refresh (it does not subscribe to engine events). The
engine's `EventBroadcaster` (`src/engine/events.rs`) exposes `emit()` and
is wired through the worker pool as a forward-looking seam, but the TUI
currently polls the store. Each refresh recomputes the visualizer's
target state from the fresh queue snapshot.
