//! TUI entry point. Wires PID lock + recovery sweep + `EnginePool` +
//! ratatui render loop into one orderly sequence (per `docs/07-cli.md`'s
//! startup spec).
//!
//! The lifecycle:
//!
//! 1. Acquire `tui.pid` flock; refuse with exit 4 if held.
//! 2. Open the store; run migrations.
//! 3. Run startup sweeps (dead claims, abandoned conflict sessions).
//! 4. Build the `EnginePool`; call `reconcile()` once before the first
//!    render.
//! 5. Enter the render loop. Each iteration:
//!    - Compute the inter-frame delta; advance + draw the Braille glyph
//!      animation with ratatui.
//!    - Throttled (~5 Hz): reconcile the pool + pull a fresh entry list.
//!    - Poll keyboard for up to one frame interval.
//! 6. On exit: signal pool shutdown; join workers; drop the lock.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use crossterm::execute;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use thiserror::Error;
use time::OffsetDateTime;

use crate::agents::tmux::{ProcessTmux, TmuxOps};
use crate::agents::{resolve, DefaultAgentRegistry};
use crate::core::agent_backend::AgentBackend;
use crate::core::ids::{ConflictSessionId, QueueEntryId, RepoId};
use crate::core::ports::{AgentRegistry, Clock, EntryFilter, GitOps, QueueStore};
use crate::core::queue::{
    MergeFailureReason, QueueEntry, QueueEntryDetailStatus, QueueEntryDetails, QueueStatus,
    StepOutcome,
};
use crate::core::repo::{RegisteredRepo, RepoCiConfig};
use crate::engine::events::EventBroadcaster;
use crate::engine::pool::{EnginePool, PoolDeps};
use crate::engine::recovery;
use crate::error::Error;
use crate::git::ProcessGit;
use crate::paths;
use crate::store::{SqliteStore, SystemClock, TuiLock, TuiLockError};
use crate::tui::app::{AppState, MAX_ENTRIES_VISIBLE};
use crate::tui::events::{poll_action, TuiAction};
use crate::tui::ui;

/// Top-level failure modes the CLI maps to documented exit codes.
#[derive(Debug, Error)]
pub enum RunError {
    #[error("another MergeQueue TUI is already running (pid {pid})")]
    LockHeld { pid: u32 },

    #[error("stdin/stdout is not a TTY; the TUI requires an interactive terminal.")]
    NotATty,

    #[error("{0}")]
    Internal(#[from] Error),
}

impl RunError {
    /// CLI exit code (per `docs/07-cli.md`).
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::LockHeld { .. } => 4,
            Self::NotATty => 2,
            Self::Internal(_) => 10,
        }
    }
}

/// Frame interval for the render loop. ~120 fps; the zoom is advanced by real
/// elapsed `dt`, so a faster tick smooths the motion without speeding it up.
/// Key handling shares this clock, but a key event wakes the poll immediately.
const TICK: Duration = Duration::from_nanos(8_333_333);

/// How often we touch the store + reconcile the pool. These are the
/// only expensive, non-animation parts of the loop, so we throttle them
/// to ~5 Hz instead of running them every frame. The animation still
/// advances at the full frame rate in between.
const DATA_REFRESH: Duration = Duration::from_millis(200);

/// Run the TUI to completion. Returns `Ok(())` on clean exit.
pub fn run(demo: bool) -> Result<(), RunError> {
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

    // 2. Store + recovery. Demo mode intentionally avoids the store and
    //    worker pool; it is a pure visual harness for tuning the TUI.
    let store: Arc<dyn QueueStore> =
        Arc::new(SqliteStore::open(&paths::sqlite_path(&state_root)).map_err(RunError::Internal)?);

    // 3. Demo mode is a pure visual harness for tuning the TUI; it skips
    //    the store and worker pool.
    if demo {
        let (repos, entries) = demo_data();
        return render_loop(None, None, Some((repos, entries)));
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
    let res = render_loop(Some(&store), Some(&pool), None);

    // 7. Shutdown.
    pool.shutdown_and_join();
    res
}

fn demo_data() -> (Vec<RegisteredRepo>, Vec<QueueEntry>) {
    let now = OffsetDateTime::now_utc();
    let repos = [
        ("atlas-web", "main", AgentBackend::Opencode),
        ("billing-core", "main", AgentBackend::Opencode),
        ("vector-search", "master", AgentBackend::Opencode),
        ("mobile-wallet", "trunk", AgentBackend::Opencode),
        ("infra-platform", "main", AgentBackend::Opencode),
        ("docs-site", "main", AgentBackend::Opencode),
    ]
    .into_iter()
    .map(|(name, default_branch, agent_backend)| {
        let id = RepoId::new();
        (
            id,
            RegisteredRepo {
                id,
                root_path: std::path::PathBuf::from("/work/demo").join(name),
                default_branch: default_branch.to_string(),
                ci: RepoCiConfig::default(),
                agent_backend,
                created_at: now - time::Duration::days(42),
                updated_at: now - time::Duration::minutes(2),
            },
        )
    })
    .collect::<Vec<_>>();

    struct DemoSpec<'a> {
        repo_index: usize,
        status: QueueStatus,
        branch: &'a str,
        target: &'a str,
        outcome: Option<StepOutcome>,
        failure_reason: Option<MergeFailureReason>,
        message: Option<&'a str>,
        headline: &'a str,
        details: &'a [(QueueEntryDetailStatus, &'a str, &'a str)],
    }

    let specs = [
        DemoSpec {
            repo_index: 0,
            status: QueueStatus::NeedsHelp,
            branch: "feat/checkout-risk-banner",
            target: "main",
            outcome: Some(StepOutcome::RebaseConflict),
            failure_reason: Some(MergeFailureReason::RebaseUnresolvable),
            message: Some("agent paused on route ownership conflict"),
            headline: "blocked while replaying checkout UI changes onto latest main",
            details: &[
                (
                    QueueEntryDetailStatus::Success,
                    "precheck",
                    "target worktree clean",
                ),
                (
                    QueueEntryDetailStatus::Success,
                    "rebase",
                    "37 commits replayed cleanly",
                ),
                (
                    QueueEntryDetailStatus::Blocked,
                    "conflict",
                    "app/routes/checkout.tsx and components/RiskBanner.tsx both changed",
                ),
                (
                    QueueEntryDetailStatus::Running,
                    "agent",
                    "opencode session mergequeue-atlas-web-42 waiting for product call",
                ),
            ],
        },
        DemoSpec {
            repo_index: 1,
            status: QueueStatus::NeedsHelp,
            branch: "fix/refund-idempotency-window",
            target: "main",
            outcome: Some(StepOutcome::TestFailed),
            failure_reason: Some(MergeFailureReason::CITestsFailed),
            message: Some("integration test failure needs a decision"),
            headline: "tests disagree about refund retry semantics around midnight UTC",
            details: &[
                (
                    QueueEntryDetailStatus::Success,
                    "rebase",
                    "clean over 8 upstream commits",
                ),
                (
                    QueueEntryDetailStatus::Success,
                    "lint",
                    "cargo clippy --all-targets passed",
                ),
                (
                    QueueEntryDetailStatus::Blocked,
                    "test",
                    "refunds::idempotency_window_allows_duplicate_after_cutoff failed",
                ),
                (
                    QueueEntryDetailStatus::Info,
                    "note",
                    "Claude Code suggested widening the cutoff but left it for review",
                ),
            ],
        },
        DemoSpec {
            repo_index: 2,
            status: QueueStatus::Rebasing,
            branch: "feat/hnsw-filter-pushdown",
            target: "master",
            outcome: Some(StepOutcome::PrecheckOk),
            failure_reason: None,
            message: Some("rebasing candidate generation changes"),
            headline: "replaying vector planner work before running perf suite",
            details: &[
                (
                    QueueEntryDetailStatus::Success,
                    "claim",
                    "worker 18421 claimed repo queue slot",
                ),
                (
                    QueueEntryDetailStatus::Success,
                    "precheck",
                    "no local modifications in target worktree",
                ),
                (
                    QueueEntryDetailStatus::Running,
                    "rebase",
                    "applying 12/19 commits onto origin/master",
                ),
                (
                    QueueEntryDetailStatus::Pending,
                    "ci",
                    "benchmark smoke will run after rebase",
                ),
            ],
        },
        DemoSpec {
            repo_index: 3,
            status: QueueStatus::CIRunning,
            branch: "feat/offline-passkey-unlock",
            target: "trunk",
            outcome: Some(StepOutcome::RebaseOk),
            failure_reason: None,
            message: Some("CI running on rebased mobile wallet branch"),
            headline: "device auth work is through rebase and waiting on CI",
            details: &[
                (
                    QueueEntryDetailStatus::Success,
                    "precheck",
                    "target clean and branch reachable",
                ),
                (
                    QueueEntryDetailStatus::Success,
                    "rebase",
                    "rebased onto trunk at 9f42c18",
                ),
                (
                    QueueEntryDetailStatus::Running,
                    "ios",
                    "xcodebuild test on iPhone 15 simulator",
                ),
                (
                    QueueEntryDetailStatus::Running,
                    "android",
                    "gradle connectedCheck shard 2/4",
                ),
            ],
        },
        DemoSpec {
            repo_index: 4,
            status: QueueStatus::Merging,
            branch: "chore/terraform-module-pinning",
            target: "main",
            outcome: Some(StepOutcome::BuildPassed),
            failure_reason: None,
            message: Some("fast-forwarding infra module locks"),
            headline: "all policy checks passed; updating main by fast-forward",
            details: &[
                (
                    QueueEntryDetailStatus::Success,
                    "rebase",
                    "already up to date with origin/main",
                ),
                (
                    QueueEntryDetailStatus::Success,
                    "plan",
                    "terraform plan produced no destructive actions",
                ),
                (
                    QueueEntryDetailStatus::Success,
                    "build",
                    "OPA policy bundle passed 128 checks",
                ),
                (
                    QueueEntryDetailStatus::Running,
                    "merge",
                    "git push origin chore/terraform-module-pinning:main",
                ),
            ],
        },
        DemoSpec {
            repo_index: 5,
            status: QueueStatus::Queued,
            branch: "docs/launch-week-case-study",
            target: "main",
            outcome: None,
            failure_reason: None,
            message: Some("next docs-site branch in FIFO order"),
            headline: "queued behind one active docs-site change",
            details: &[
                (
                    QueueEntryDetailStatus::Pending,
                    "claim",
                    "waiting for docs-site worker to become idle",
                ),
                (
                    QueueEntryDetailStatus::Pending,
                    "precheck",
                    "will verify local target before rebase",
                ),
                (
                    QueueEntryDetailStatus::Info,
                    "preview",
                    "branch updates customer story pages and homepage CTA copy",
                ),
            ],
        },
        DemoSpec {
            repo_index: 0,
            status: QueueStatus::Queued,
            branch: "fix/session-refresh-loop",
            target: "main",
            outcome: None,
            failure_reason: None,
            message: Some("waiting behind checkout-risk-banner"),
            headline: "queued for atlas-web after the current NeedsHelp item resolves",
            details: &[
                (
                    QueueEntryDetailStatus::Pending,
                    "claim",
                    "repo worker is busy with older queue item",
                ),
                (
                    QueueEntryDetailStatus::Info,
                    "change",
                    "narrows refresh token retry backoff in browser tabs",
                ),
                (
                    QueueEntryDetailStatus::Pending,
                    "ci",
                    "playwright auth suite will run after rebase",
                ),
            ],
        },
        DemoSpec {
            repo_index: 1,
            status: QueueStatus::Queued,
            branch: "feat/invoice-adjustment-api",
            target: "main",
            outcome: None,
            failure_reason: None,
            message: Some("third in billing-core queue"),
            headline: "API addition queued behind refund idempotency fix",
            details: &[
                (
                    QueueEntryDetailStatus::Pending,
                    "claim",
                    "billing-core queue is preserving FIFO",
                ),
                (
                    QueueEntryDetailStatus::Info,
                    "migration",
                    "adds adjustment_reason enum and audit trail table",
                ),
                (
                    QueueEntryDetailStatus::Pending,
                    "tests",
                    "contract tests will run in isolated postgres",
                ),
            ],
        },
        DemoSpec {
            repo_index: 2,
            status: QueueStatus::Merged,
            branch: "fix/shard-compaction-throttle",
            target: "master",
            outcome: Some(StepOutcome::FastForwardOk),
            failure_reason: None,
            message: Some("merged 6 minutes ago"),
            headline: "merged cleanly after rebase and search soak test",
            details: &[
                (
                    QueueEntryDetailStatus::Success,
                    "rebase",
                    "clean over origin/master",
                ),
                (
                    QueueEntryDetailStatus::Success,
                    "tests",
                    "unit, property, and soak smoke passed",
                ),
                (
                    QueueEntryDetailStatus::Success,
                    "merge",
                    "fast-forwarded master to 41d8a7c",
                ),
            ],
        },
        DemoSpec {
            repo_index: 4,
            status: QueueStatus::Merged,
            branch: "fix/k8s-readiness-probe",
            target: "main",
            outcome: Some(StepOutcome::FastForwardOk),
            failure_reason: None,
            message: Some("merged after policy check"),
            headline: "readiness probe timeout fix is on main",
            details: &[
                (QueueEntryDetailStatus::Success, "precheck", "target clean"),
                (
                    QueueEntryDetailStatus::Success,
                    "ci",
                    "kind smoke cluster healthy",
                ),
                (
                    QueueEntryDetailStatus::Success,
                    "merge",
                    "main advanced with no force push",
                ),
            ],
        },
        DemoSpec {
            repo_index: 3,
            status: QueueStatus::Failed,
            branch: "feat/apple-watch-balance-widget",
            target: "trunk",
            outcome: Some(StepOutcome::BuildFailed),
            failure_reason: Some(MergeFailureReason::CIBuildFailed),
            message: Some("watchOS compile failed"),
            headline: "build failed after a clean rebase; left branch untouched",
            details: &[
                (
                    QueueEntryDetailStatus::Success,
                    "rebase",
                    "clean onto trunk",
                ),
                (QueueEntryDetailStatus::Success, "swiftlint", "0 violations"),
                (
                    QueueEntryDetailStatus::Blocked,
                    "build",
                    "WatchBalanceWidget.swift cannot find BalanceTimelineProvider",
                ),
                (
                    QueueEntryDetailStatus::Info,
                    "log",
                    "ci/mobile-wallet/watchos-build.log",
                ),
            ],
        },
        DemoSpec {
            repo_index: 5,
            status: QueueStatus::Cancelled,
            branch: "wip/migration-guide-v2",
            target: "main",
            outcome: Some(StepOutcome::UserCancelled),
            failure_reason: Some(MergeFailureReason::UserCancelled),
            message: Some("cancelled by release manager"),
            headline: "cancelled before claim because the docs were split into smaller PRs",
            details: &[
                (
                    QueueEntryDetailStatus::Info,
                    "request",
                    "release manager cancelled stale WIP branch",
                ),
                (
                    QueueEntryDetailStatus::Success,
                    "cleanup",
                    "queue row marked terminal; no worker claim existed",
                ),
            ],
        },
    ];

    let entries = specs
        .into_iter()
        .zip(0_i64..)
        .map(|(spec, i)| {
            let (repo_id, repo) = &repos[spec.repo_index];
            let started = matches!(
                spec.status,
                QueueStatus::Rebasing
                    | QueueStatus::CIRunning
                    | QueueStatus::Merging
                    | QueueStatus::NeedsHelp
                    | QueueStatus::Merged
                    | QueueStatus::Failed
                    | QueueStatus::Cancelled
            )
            .then_some(now - time::Duration::minutes(20 - i));
            let finished = spec
                .status
                .is_terminal()
                .then_some(now - time::Duration::minutes(8 - (i % 3)));
            let mut details = QueueEntryDetails::new(spec.headline);
            for (status, title, detail) in spec.details {
                details.push(*status, *title, Some((*detail).to_string()));
            }

            QueueEntry {
                id: QueueEntryId::new(),
                repo_id: *repo_id,
                source_worktree: repo.root_path.join(spec.branch.replace('/', "-")),
                source_branch: spec.branch.to_string(),
                target_branch: spec.target.to_string(),
                status: spec.status,
                last_outcome: spec.outcome,
                enqueued_at: now - time::Duration::minutes(60 - i * 4),
                started_at: started,
                finished_at: finished,
                failure_reason: spec.failure_reason,
                ci_log_dir: Some(repo.root_path.join(".mergequeue/demo-ci")),
                merge_log_path: Some(repo.root_path.join(".mergequeue/merge.log")),
                conflict_session_id: (spec.status == QueueStatus::NeedsHelp)
                    .then(ConflictSessionId::new),
                message: spec.message.map(str::to_string),
                details: Some(details),
                claimed_by_pid: None,
                claimed_at: None,
            }
        })
        .collect();
    let repos = repos.into_iter().map(|(_, repo)| repo).collect();
    (repos, entries)
}

fn render_loop(
    store: Option<&Arc<dyn QueueStore>>,
    pool: Option<&EnginePool>,
    demo_data: Option<(Vec<RegisteredRepo>, Vec<QueueEntry>)>,
) -> Result<(), RunError> {
    // Enter alt screen + raw mode. Restoration on drop.
    let mut stdout = io::stdout();
    let mut alt = AltScreenGuard::enter(&mut stdout)
        .map_err(|e| RunError::Internal(Error::other(format!("alt screen: {e}"))))?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)
        .map_err(|e| RunError::Internal(Error::other(format!("terminal: {e}"))))?;

    let mut app = AppState::new();

    // Real-time clocks so motion is frame-rate independent and the
    // expensive data refresh is throttled below the animation rate.
    let mut last_frame = std::time::Instant::now();
    let mut last_data_refresh: Option<std::time::Instant> = None;
    // Remembered layout: we only re-run the ratatui draw when the data
    // actually changed or the animation advanced.
    let mut last_layout: Option<ui::DrawLayout> = None;
    // Set whenever a key action mutated visible state (selection, help,
    // status toast) so the widgets redraw next frame even without a data
    // refresh. First frame is implicitly dirty (no cached layout yet).
    let mut dirty = false;
    let mut focused = true;

    loop {
        // --- Throttled data refresh + ratatui redraw (~5 Hz) ---------
        let need_data = last_data_refresh.is_none_or(|t| t.elapsed() >= DATA_REFRESH);
        if need_data {
            last_data_refresh = Some(std::time::Instant::now());

            // Pull a fresh queue snapshot, or reuse the in-memory demo
            // showcase when running `mergequeue tui --demo`.
            let entries = if let Some((repos, entries)) = demo_data.as_ref() {
                app.ingest_repos(repos);
                entries.clone()
            } else {
                let store = store.expect("store is present outside demo mode");
                let repos = store.list_repos().map_err(RunError::Internal)?;
                app.ingest_repos(&repos);
                store
                    .list_entries(EntryFilter::all())
                    .map_err(RunError::Internal)?
            };
            let total = entries.len();
            app.ingest_entries(entries);
            if total > MAX_ENTRIES_VISIBLE {
                app.set_status(format!("showing {MAX_ENTRIES_VISIBLE} of {total} entries"));
            }

            // Reconcile pool to the current repo set. Cheap when nothing
            // changed; logs to tracing when it does.
            if let Some(pool) = pool {
                let report = pool.reconcile().map_err(RunError::Internal)?;
                if !report.started.is_empty() || !report.stopped.is_empty() {
                    app.set_status(format!(
                        "workers: +{} -{} (active {})",
                        report.started.len(),
                        report.stopped.len(),
                        pool.worker_count(),
                    ));
                }
            } else {
                app.set_status("demo data · workers disabled");
            }
        }

        // Real elapsed time since the previous frame drives the zoom, so
        // the motion is smooth and correct regardless of actual fps.
        let now = std::time::Instant::now();
        let dt = if focused {
            now.duration_since(last_frame).as_secs_f32()
        } else {
            0.0
        };
        last_frame = now;
        app.frame_dt = dt;

        // Redraw the ratatui widgets when something visible changed
        // (first frame, data refresh, or a key action). The animation is
        // drawn by ratatui and advances every frame, so we must redraw
        // every frame while focused.
        if last_layout.is_none() || need_data || dirty || focused {
            dirty = false;
            let mut layout = None;
            terminal
                .draw(|f| {
                    layout = Some(ui::draw(f, &mut app));
                })
                .map_err(|e| RunError::Internal(Error::other(format!("draw: {e}"))))?;
            last_layout = layout;
        }
        let Some(_layout) = last_layout else { continue };

        if focused {
            app.advance_sprite();
        }

        // Poll for one key event. When unfocused, wake much less often;
        // focus-gained events still interrupt the poll immediately.
        let poll_timeout = if focused { TICK } else { DATA_REFRESH };
        let action = poll_action(poll_timeout)
            .map_err(|e| RunError::Internal(Error::other(format!("poll: {e}"))))?
            .unwrap_or(TuiAction::Ignore);

        match action {
            TuiAction::Ignore => {}
            TuiAction::FocusGained => {
                focused = true;
                dirty = true;
                last_frame = std::time::Instant::now();
            }
            TuiAction::FocusLost => {
                focused = false;
                dirty = false;
            }
            TuiAction::SelectDown => {
                app.move_selection_down();
                dirty = true;
            }
            TuiAction::SelectUp => {
                app.move_selection_up();
                dirty = true;
            }
            TuiAction::ToggleHelp => {
                app.toggle_help();
                dirty = true;
            }
            TuiAction::ToggleExpanded => {
                app.toggle_selected_expanded();
                if let Some(entry) = app.selected() {
                    let state = if app.selected_is_expanded() {
                        "expanded"
                    } else {
                        "collapsed"
                    };
                    app.set_status(format!("{state} {}", entry.id.short()));
                }
                dirty = true;
            }
            TuiAction::DeleteSelected => {
                if let Some(entry) = app.selected().cloned() {
                    if app.selected_is_deletable() {
                        if let Some(store) = store {
                            match store.delete_queued(entry.id) {
                                Ok(true) => {
                                    app.set_status(format!("deleted {}", entry.id.short()));
                                }
                                Ok(false) => {
                                    app.set_status(format!(
                                        "entry {} not deletable",
                                        entry.id.short()
                                    ));
                                }
                                Err(e) => app.set_status(format!("delete failed: {e}")),
                            }
                        } else {
                            app.set_status("demo data is read-only");
                        }
                    } else {
                        app.set_status("only Queued entries can be deleted");
                    }
                }
                dirty = true;
            }
            TuiAction::AttachSelected => {
                if !app.selected_needs_help() {
                    app.set_status("select a NeedsHelp entry to open its agent");
                    dirty = true;
                    continue;
                }
                let Some(entry) = app.selected().cloned() else {
                    app.set_status("no entry selected");
                    dirty = true;
                    continue;
                };
                let Some(store) = store else {
                    app.set_status("demo data has no agent session");
                    dirty = true;
                    continue;
                };
                let attach_result = (|| -> crate::error::Result<()> {
                    let repo = store
                        .get_repo(entry.repo_id)?
                        .ok_or_else(|| Error::NotFound(format!("repo {}", entry.repo_id)))?;
                    let tmux = ProcessTmux::new();
                    let agents = DefaultAgentRegistry::new(Arc::new(ProcessTmux::new()));
                    let handle =
                        resolve::conflict_session_handle(&**store, &tmux, &agents, &repo, &entry)?;
                    alt.suspend()
                        .map_err(|e| Error::other(format!("leave TUI: {e}")))?;
                    let attach = resolve::attach_tmux(&handle);
                    alt.resume()
                        .map_err(|e| Error::other(format!("resume TUI: {e}")))?;
                    attach
                })();
                match attach_result {
                    Ok(()) => app.set_status(format!("detached from agent {}", entry.id.short())),
                    Err(e) => app.set_status(format!("attach failed: {e}")),
                }
                last_layout = None;
                last_frame = std::time::Instant::now();
                focused = true;
                dirty = true;
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
                if let Some(pool) = pool {
                    pool.shutdown_token().set_hard();
                }
                break;
            }
        }
    }

    // Make the cursor visible again before we restore.
    let _ = crossterm::execute!(io::stdout(), crossterm::cursor::Show);
    Ok(())
}

/// RAII wrapper for the alternate-screen mode + raw-mode pair.
/// Disables both on drop, even on panic.
struct AltScreenGuard {
    active: bool,
}

impl AltScreenGuard {
    fn enter(stdout: &mut io::Stdout) -> io::Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        execute!(
            stdout,
            crossterm::terminal::EnterAlternateScreen,
            crossterm::event::EnableFocusChange,
            crossterm::cursor::Hide
        )?;
        Ok(Self { active: true })
    }

    fn suspend(&mut self) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }
        execute!(
            io::stdout(),
            crossterm::cursor::Show,
            crossterm::event::DisableFocusChange,
            crossterm::terminal::LeaveAlternateScreen
        )?;
        crossterm::terminal::disable_raw_mode()?;
        self.active = false;
        Ok(())
    }

    fn resume(&mut self) -> io::Result<()> {
        if self.active {
            return Ok(());
        }
        crossterm::terminal::enable_raw_mode()?;
        execute!(
            io::stdout(),
            crossterm::terminal::EnterAlternateScreen,
            crossterm::event::EnableFocusChange,
            crossterm::cursor::Hide
        )?;
        self.active = true;
        Ok(())
    }
}

impl Drop for AltScreenGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let _ = crossterm::execute!(
            io::stdout(),
            crossterm::cursor::Show,
            crossterm::event::DisableFocusChange,
            crossterm::terminal::LeaveAlternateScreen
        );
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

#[cfg(test)]
mod demo_tests {
    use super::*;

    #[test]
    fn demo_showcase_uses_multiple_repositories() {
        let (repos, entries) = demo_data();

        assert!(repos.len() >= 4);
        assert!(
            entries
                .iter()
                .map(|entry| entry.repo_id)
                .collect::<std::collections::HashSet<_>>()
                .len()
                >= 4
        );
    }

    #[test]
    fn demo_showcase_entries_are_expandable() {
        let (_repos, entries) = demo_data();

        assert!(entries.iter().all(|entry| entry
            .details
            .as_ref()
            .is_some_and(|details| details.headline.is_some() && !details.items.is_empty())));
    }
}
