# 14 — Open questions (still on the table)

These were on the punch list and I want explicit answers before we leave
the planning phase:

1. **Bundle/binary/repo name** — `mergequeue` everywhere. Confirm
   availability on GitHub org, Homebrew tap, domain (if any).
2. **License choice** — `MIT OR Apache-2.0`. Confirm.
3. **PID-file lock when stale** — `fs2::FileExt::try_lock_exclusive` on
   `tui.pid` handles this automatically (stale PID → flock succeeds).
   Confirm OK with this behavior.
4. **`StepOutcome` shape** — one flat enum in v1; split per-state if it
   gets unwieldy. Confirm.
5. **`mergequeue` bare command aliasing `mergequeue tui`** — yes.
   Confirm.
6. **Sprite frame data format** — Aseprite `json-array`. Confirm or let
   the artist choose at M5.
7. **TUI quit on in-flight merges** — soft `q` (finish current step, 60s
   timeout, then prompt) + hard `Q` (abort, mark `Cancelled
   {UncleanShutdown}`). Confirm.
8. **Notifications (`notify-rust`)** — on hold. Re-evaluate at M4 if the
   sprite alone isn't enough "needs attention" signal.
9. **Per-repo concurrency** — one worker per repo (locked). Future option:
   per-repo `max_parallel = N` for repos that have entries with disjoint
   target branches. Out of scope for v1.
10. **Sprite acquisition** — placeholder from itch.io at M4 ($5-15);
    commissioned at M5 ($200-400 from Lospec/Cara/Fiverr). Confirm
    budget range.
