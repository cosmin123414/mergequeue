//! TUI entry point. Wires PID lock + Kitty probe + recovery sweep +
//! `EnginePool` + ratatui render loop into one orderly sequence (per
//! `docs/07-cli.md`'s startup spec).
//!
//! The lifecycle:
//!
//! 1. Acquire `tui.pid` flock; refuse with exit 4 if held.
//! 2. Open the store; run migrations.
//! 3. Probe for Kitty graphics support; refuse with exit 2 if absent.
//! 4. Run startup sweeps (dead claims, abandoned conflict sessions).
//! 5. Build the `EnginePool`; call `reconcile()` once before the first
//!    render.
//! 6. Enter the render loop. Each iteration:
//!    - Poll keyboard for up to `tick`.
//!    - Reconcile the pool (cheap when nothing changed).
//!    - Pull a fresh entry list from the store.
//!    - Draw a frame with ratatui.
//!    - Emit a Kitty place command for the current sprite frame.
//!    - Advance the sprite tick.
//! 7. On exit: signal pool shutdown; join workers; cleanup sprite;
//!    drop the lock.

use std::io::{self, Write};
use std::sync::Arc;
use std::time::Duration;

use crossterm::execute;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use thiserror::Error;

use crate::agents::tmux::{ProcessTmux, TmuxOps};
use crate::agents::DefaultAgentRegistry;
use crate::core::ports::{AgentRegistry, Clock, EntryFilter, GitOps, QueueStore};
use crate::engine::events::EventBroadcaster;
use crate::engine::pool::{EnginePool, PoolDeps};
use crate::engine::recovery;
use crate::error::Error;
use crate::git::ProcessGit;
use crate::paths;
use crate::store::{SqliteStore, SystemClock, TuiLock, TuiLockError};
use crate::tui::app::{AppState, MAX_ENTRIES_VISIBLE};
use crate::tui::capability::{probe_stdin, KittySupport, PROBE_TIMEOUT};
use crate::tui::events::{poll_action, TuiAction};
use crate::tui::sprite::SpriteRenderer;
use crate::tui::ui;

/// Top-level failure modes the CLI maps to documented exit codes.
#[derive(Debug, Error)]
pub enum RunError {
    #[error("another MergeSmith TUI is already running (pid {pid})")]
    LockHeld { pid: u32 },

    #[error("stdin/stdout is not a TTY; the TUI requires an interactive terminal.")]
    NotATty,

    #[error(
        "terminal does not support the Kitty graphics protocol. Try Ghostty, Kitty, or WezTerm."
    )]
    KittyUnsupported,

    #[error("{0}")]
    Internal(#[from] Error),
}

impl RunError {
    /// CLI exit code (per `docs/07-cli.md`).
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::LockHeld { .. } => 4,
            Self::NotATty | Self::KittyUnsupported => 2,
            Self::Internal(_) => 10,
        }
    }
}

/// Tick interval for the render loop. 12 fps is the default sprite
/// speed; we run the whole loop at this rate so animation and key
/// handling share one clock.
const TICK: Duration = Duration::from_millis(80);

/// Run the TUI to completion. Returns `Ok(())` on clean exit.
pub fn run() -> Result<(), RunError> {
    // 0. TTY guard. Without a terminal on stdin AND stdout we'd
    //    just blow up in `enable_raw_mode` with a confusing errno.
    //    Pre-check produces a friendly message + the right exit code.
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(RunError::NotATty);
    }

    // 1. State root + lock.
    let state_root = paths::state_root().map_err(RunError::Internal)?;
    paths::ensure_state_root(&state_root).map_err(RunError::Internal)?;
    let _tui_lock = TuiLock::acquire(&paths::tui_pid_path(&state_root)).map_err(|e| match e {
        TuiLockError::AlreadyHeld { pid } => RunError::LockHeld { pid },
        TuiLockError::Io(io) => RunError::Internal(Error::Io(io)),
    })?;

    // 2. Store + recovery.
    let store: Arc<dyn QueueStore> =
        Arc::new(SqliteStore::open(&paths::sqlite_path(&state_root)).map_err(RunError::Internal)?);

    // 3. Capability probe BEFORE we enter the alt screen. If we
    //    don't support Kitty, leave the terminal untouched and bail.
    if probe_stdin(PROBE_TIMEOUT).map_err(RunError::Internal)? != KittySupport::Supported {
        return Err(RunError::KittyUnsupported);
    }

    // 4. Startup sweeps. Dead-claim sweep runs always; abandoned
    //    conflict sessions sweep needs tmux on the host (best-effort).
    let _swept = recovery::sweep_dead_claims(&*store, &[]).map_err(RunError::Internal)?;
    let tmux_ops: Arc<dyn TmuxOps> = Arc::new(ProcessTmux::new());
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let _orphan_sessions =
        recovery::sweep_abandoned_conflict_sessions(&*store, &*tmux_ops, &*clock)
            .map_err(RunError::Internal)?;

    // 5. EnginePool.
    let git: Arc<dyn GitOps> = Arc::new(ProcessGit::new());
    let agents: Arc<dyn AgentRegistry> = Arc::new(DefaultAgentRegistry::new(tmux_ops.clone()));
    let events = Arc::new(EventBroadcaster::new());
    let pool = EnginePool::new(PoolDeps {
        store: store.clone(),
        git,
        clock: clock.clone(),
        events,
        runs_dir: paths::runs_dir(&state_root),
        poll_interval: Duration::from_millis(250),
        tmux: tmux_ops.clone(),
        agents,
        tmux_session_pid: std::process::id(),
    });
    pool.reconcile().map_err(RunError::Internal)?;

    // 6. Render loop. From here, panics MUST restore the terminal —
    //    we use `scopeguard`-style explicit cleanup at the bottom of
    //    `render_loop`.
    let res = render_loop(&store, &pool);

    // 7. Shutdown.
    pool.shutdown_and_join();
    res
}

fn render_loop(store: &Arc<dyn QueueStore>, pool: &EnginePool) -> Result<(), RunError> {
    // Enter alt screen + raw mode. Restoration on drop.
    let mut stdout = io::stdout();
    crossterm::terminal::enable_raw_mode()
        .map_err(|e| RunError::Internal(Error::other(format!("raw mode: {e}"))))?;
    let _alt = AltScreenGuard::enter(&mut stdout)
        .map_err(|e| RunError::Internal(Error::other(format!("alt screen: {e}"))))?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)
        .map_err(|e| RunError::Internal(Error::other(format!("terminal: {e}"))))?;

    let tmux_passthrough = std::env::var_os("TMUX").is_some();
    let mut sprite = SpriteRenderer::new(Box::new(io::stdout()), tmux_passthrough)
        .map_err(RunError::Internal)?;

    let mut app = AppState::new();

    loop {
        // Pull a fresh queue snapshot.
        let entries = store
            .list_entries(EntryFilter::all())
            .map_err(RunError::Internal)?;
        let total = entries.len();
        app.ingest_entries(entries);
        if total > MAX_ENTRIES_VISIBLE {
            app.set_status(format!("showing {MAX_ENTRIES_VISIBLE} of {total} entries"));
        }

        // Reconcile pool to the current repo set. Cheap when nothing
        // changed; logs to tracing when it does.
        let report = pool.reconcile().map_err(RunError::Internal)?;
        if !report.started.is_empty() || !report.stopped.is_empty() {
            app.set_status(format!(
                "workers: +{} -{} (active {})",
                report.started.len(),
                report.stopped.len(),
                pool.worker_count(),
            ));
        }

        // Draw ratatui frame.
        let mut layout = None;
        terminal
            .draw(|f| {
                layout = Some(ui::draw(f, &mut app));
            })
            .map_err(|e| RunError::Internal(Error::other(format!("draw: {e}"))))?;
        let Some(layout) = layout else { continue };

        // Position the cursor at the sprite's cell origin and emit
        // the Kitty place command. ratatui leaves the cursor at the
        // bottom-right after a draw; we move it explicitly.
        let mut so = io::stdout();
        crossterm::queue!(
            so,
            crossterm::cursor::MoveTo(layout.sprite.x, layout.sprite.y)
        )
        .map_err(|e| RunError::Internal(Error::other(format!("cursor: {e}"))))?;
        sprite
            .place(
                app.sprite_state,
                app.sprite_tick,
                u32::from(layout.sprite.width.saturating_sub(2)),
                u32::from(layout.sprite.height),
            )
            .map_err(RunError::Internal)?;
        // Hide the cursor so it doesn't blink on top of the sprite.
        crossterm::queue!(so, crossterm::cursor::Hide)
            .map_err(|e| RunError::Internal(Error::other(format!("hide cursor: {e}"))))?;
        so.flush()
            .map_err(|e| RunError::Internal(Error::other(format!("flush: {e}"))))?;

        app.advance_sprite();

        // Poll for one key event up to TICK.
        let action = poll_action(TICK)
            .map_err(|e| RunError::Internal(Error::other(format!("poll: {e}"))))?
            .unwrap_or(TuiAction::Ignore);

        match action {
            TuiAction::Ignore => {}
            TuiAction::SelectDown => app.move_selection_down(),
            TuiAction::SelectUp => app.move_selection_up(),
            TuiAction::ToggleHelp => app.toggle_help(),
            TuiAction::DeleteSelected => {
                if let Some(entry) = app.selected().cloned() {
                    if app.selected_is_deletable() {
                        match store.delete_queued(entry.id) {
                            Ok(true) => app.set_status(format!("deleted {}", entry.id.short())),
                            Ok(false) => {
                                app.set_status(format!("entry {} not deletable", entry.id.short()));
                            }
                            Err(e) => app.set_status(format!("delete failed: {e}")),
                        }
                    } else {
                        app.set_status("only Queued entries can be deleted");
                    }
                }
            }
            TuiAction::QuitSoft => {
                app.set_status("shutting down (soft)…");
                break;
            }
            TuiAction::QuitHard => {
                // Hard quit asks the pool to escalate immediately. We
                // still join cleanly in `run()` after the loop exits;
                // for M4 the practical difference is that we don't
                // pause to ask "are you sure?" — Q is the answer.
                pool.shutdown_token().set_hard();
                break;
            }
        }
    }

    // Sprite delete is best-effort and happens automatically when
    // `sprite` drops; we also pre-flight it to avoid a flash of the
    // last frame after the alt-screen leaves.
    sprite.cleanup().map_err(RunError::Internal)?;
    drop(sprite);

    // Make the cursor visible again before we restore.
    let _ = crossterm::execute!(io::stdout(), crossterm::cursor::Show);
    Ok(())
}

/// RAII wrapper for the alternate-screen mode + raw-mode pair.
/// Disables both on drop, even on panic.
struct AltScreenGuard;

impl AltScreenGuard {
    fn enter(stdout: &mut io::Stdout) -> io::Result<Self> {
        execute!(
            stdout,
            crossterm::terminal::EnterAlternateScreen,
            crossterm::cursor::Hide
        )?;
        Ok(Self)
    }
}

impl Drop for AltScreenGuard {
    fn drop(&mut self) {
        let _ = crossterm::execute!(
            io::stdout(),
            crossterm::cursor::Show,
            crossterm::terminal::LeaveAlternateScreen
        );
        let _ = crossterm::terminal::disable_raw_mode();
    }
}
