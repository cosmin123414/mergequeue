# 08 — TUI and sprite

## Layout

```
┌─ MergeSmith ────────────────────────────────────────────────────┐
│ ╔═══════════╗   Queue                                           │
│ ║   [PNG]   ║   ┌──────────────────────────────────────────┐    │
│ ║  64×64    ║   │ ⚒ Rebasing  feat/auth → main  agent-orc │    │
│ ║  sprite   ║   │ ⏳ CI       feat/ui   → main  mergesm.  │    │
│ ║  here     ║   │ ⏳ Queued   chore     → main  mergesm.  │    │
│ ╚═══════════╝   │ ⚠ Help     feat/api  → dev   other     │    │
│                 │ ✓ Merged   fix/typo  → main  agent-orc │    │
│ Hammering…      └──────────────────────────────────────────┘    │
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

## Sprite rendering

Kitty graphics protocol only. Reserved cell rectangle in the top-left for
the sprite — ratatui paints the full screen, then the sprite renderer
"punches through" with `a=p` placement commands at the reserved rect's
cell origin.

### Sprite states

```
Idle      8-frame loop, leaning on anvil          (queue empty)
Working   8-frame hammering loop                  (Rebasing / Merging)
CI        8-frame bellows loop, forge glows       (CIRunning)
NeedsHelp 3-frame scratch-head loop               (any NeedsHelp present)
Celebrate 4-frame one-shot, fist-pump             (one-shot on Merged)
Failed    2-frame hammer-dropped pose             (one-shot on Failed)
```

State priority when multiple entries are active across repos:
NeedsHelp > Working > CI > Idle. Celebrate and Failed are one-shots that
overlay any current state for ~1 second.

### Asset pipeline

Aseprite source in `art/blacksmith/blacksmith.aseprite`. Build step (a
`justfile` recipe) runs:

```sh
aseprite -b blacksmith.aseprite \
   --sheet assets/sprites/blacksmith.png \
   --data  assets/sprites/blacksmith.json \
   --format json-array
```

`json-array` chosen for simpler animation iteration. Exports are
committed; the recipe is for re-export. The PNG + JSON are embedded with
`include_bytes!` / `include_str!`.

### Frame timing

Configurable via `config.toml`:

```toml
[tui]
sprite_fps = 12
sprite_scale = 1
celebrate_on_merge = true
```

Default 12 fps for chunky pixel-art feel. The sprite ticker is independent
of the ratatui render tick — sprite animation is smooth even when no
queue events are coming in.

### Kitty protocol bytes

Implemented in `src/tui/sprite/kitty.rs`. Three operations:

- `transmit_image(id, png_bytes)` — uploads the full sprite sheet once at
  startup.
- `place_region(id, cell_x, cell_y, src_x, src_y, w, h)` — show frame N at
  position; called every frame transition.
- `delete_image(id)` — cleanup on quit.

Base64-encoded payloads, chunked at 4096 bytes per protocol envelope, all
wrapped in `\x1b_G…\x1b\\`. See the Kitty protocol spec for the wire details.

### Capability detection

On `mergesmith tui` startup, before any rendering:

1. Switch terminal to raw mode.
2. Emit a tiny query: `\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\`.
3. Read response with 200ms timeout.
4. If response starts with `\x1b_Gi=31;OK\x1b\\`, Kitty support confirmed.
5. Otherwise, restore terminal, print the unsupported-terminal message,
   exit 2.

### tmux passthrough

If `$TMUX` is set, prepend `\x1bPtmux;\x1b` and append `\x1b\\` to every
Kitty escape. Requires `set -g allow-passthrough on` in the user's tmux
config. `doctor` checks this and prints a hint if missing.

## Event subscription

The TUI subscribes to the engine's `EventBroadcaster` (a
crossbeam-channel `Receiver<QueueEvent>` per subscriber). Render loop is
driven by:

- Sprite ticker (FPS-driven, mostly cosmetic)
- ratatui ticker (every 200ms or on event)
- Crossterm key events (instant)

A new event triggers a redraw of the queue table + status line; the
sprite state machine recomputes its target state from the new queue
snapshot.
