//! TUI application state. Owns:
//!
//! - The current queue snapshot (refreshed every poll tick).
//! - The selection state for the queue table.
//! - The sprite ticker (a monotonically-increasing tick that drives
//!   animation independently of queue events).
//! - The latest few log lines for the active entry (M4: placeholder
//!   single line; M5+ can tail CI logs).
//! - The shutdown intention (`q` → soft, `Q` → hard, exit immediately
//!   without user prompt).

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use ratatui::widgets::TableState;

use crate::core::ids::{QueueEntryId, RepoId};
use crate::core::queue::{QueueEntry, QueueStatus};
use crate::core::repo::RegisteredRepo;
use crate::tui::sprite::state::{compute_sprite_state, QueueSnapshot, SpriteState};
use crate::tui::sprite::{DotBitmap, PenroseAnimator};

pub struct AppState {
    /// Latest list of entries from the store, sorted by `enqueued_at`
    /// then `repo_id`. Capped at `MAX_ENTRIES_VISIBLE` for the table.
    pub entries: Vec<QueueEntry>,
    /// Display label for each registered repo, derived from its root path
    /// leaf. Bucket rows use this to show `repo:branch` instead of noisy
    /// absolute paths.
    pub repo_labels: HashMap<RepoId, String>,
    pub table: TableState,
    pub expanded_entry_ids: HashSet<QueueEntryId>,
    pub sprite_tick: u64,
    /// Computed sprite state for the current snapshot. Cached so we
    /// only recompute when entries change.
    pub sprite_state: SpriteState,
    /// Most-recent toast / status line.
    pub status_line: String,
    /// Wall-clock timestamp the TUI started; used for the title bar.
    pub started_at: Instant,
    /// Set when a `?` overlay is being shown.
    pub help_visible: bool,
    /// Tiling animator. Owned here so the glyph path can advance + sample
    /// it inside the ratatui draw closure.
    pub animator: PenroseAnimator,
    /// Seconds elapsed since the previous frame; fed to the animator when
    /// drawing glyphs. Set by `run.rs` each iteration.
    pub frame_dt: f32,
    /// Previous glyph dot field, used only by text-mode rendering to decay
    /// disappearing dots over a few frames instead of letting them sparkle.
    pub glyph_prev_bitmap: Option<DotBitmap>,
}

pub const MAX_ENTRIES_VISIBLE: usize = 200;

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    /// Construct the TUI state. The centerpiece animation is rendered as
    /// Braille text glyphs, which works in any terminal.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            repo_labels: HashMap::new(),
            table: TableState::default(),
            expanded_entry_ids: HashSet::new(),
            sprite_tick: 0,
            sprite_state: SpriteState::Idle,
            status_line: "ready".into(),
            started_at: Instant::now(),
            help_visible: false,
            animator: PenroseAnimator::new(),
            frame_dt: 0.0,
            glyph_prev_bitmap: None,
        }
    }

    /// Replace the entry list and recompute the sprite state. Caller
    /// is responsible for fetching from the store.
    pub fn ingest_entries(&mut self, mut entries: Vec<QueueEntry>) {
        // FIFO by enqueue time. Two entries with the same enqueue
        // second sort by id, which is a UUIDv4 — random but stable.
        entries.sort_by(|a, b| {
            a.enqueued_at
                .cmp(&b.enqueued_at)
                .then_with(|| a.id.to_string().cmp(&b.id.to_string()))
        });
        if entries.len() > MAX_ENTRIES_VISIBLE {
            entries.truncate(MAX_ENTRIES_VISIBLE);
        }
        let snap = QueueSnapshot::from_entries(&entries);
        self.sprite_state = compute_sprite_state(&snap);
        // Preserve selection across refreshes:
        // - empty list → drop selection.
        // - had a selection that's now past the end → clamp to last.
        // - no selection yet, non-empty list → select head.
        if entries.is_empty() {
            self.table.select(None);
        } else if let Some(sel) = self.table.selected() {
            let max = entries.len() - 1;
            if sel > max {
                self.table.select(Some(max));
            }
        } else {
            self.table.select(Some(0));
        }
        let visible_ids: HashSet<QueueEntryId> = entries.iter().map(|entry| entry.id).collect();
        self.expanded_entry_ids
            .retain(|id| visible_ids.contains(id));
        self.entries = entries;
    }

    pub fn ingest_repos(&mut self, repos: &[RegisteredRepo]) {
        self.repo_labels = repos
            .iter()
            .map(|repo| {
                let label =
                    if let Some(name) = repo.root_path.file_name().and_then(|name| name.to_str()) {
                        name.to_string()
                    } else {
                        repo.root_path.to_string_lossy().to_string()
                    };
                (repo.id, label)
            })
            .collect();
    }

    pub fn advance_sprite(&mut self) {
        self.sprite_tick = self.sprite_tick.wrapping_add(1);
    }

    pub fn move_selection_down(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let next = match self.table.selected() {
            Some(i) if i + 1 < self.entries.len() => i + 1,
            Some(i) => i, // already at last
            None => 0,
        };
        self.table.select(Some(next));
    }

    pub fn move_selection_up(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let prev = match self.table.selected() {
            Some(0) | None => 0,
            Some(i) => i - 1,
        };
        self.table.select(Some(prev));
    }

    /// Returns the currently-selected entry, if any.
    pub fn selected(&self) -> Option<&QueueEntry> {
        self.table.selected().and_then(|i| self.entries.get(i))
    }

    /// Whether the selected entry is in a state that the TUI can act
    /// on for delete.
    pub fn selected_is_deletable(&self) -> bool {
        self.selected()
            .is_some_and(|e| e.status == QueueStatus::Queued)
    }

    pub fn selected_needs_help(&self) -> bool {
        self.selected()
            .is_some_and(|e| e.status == QueueStatus::NeedsHelp)
    }

    pub fn toggle_selected_expanded(&mut self) {
        let Some(id) = self.selected().map(|entry| entry.id) else {
            return;
        };
        if !self.expanded_entry_ids.insert(id) {
            self.expanded_entry_ids.remove(&id);
        }
    }

    pub fn selected_is_expanded(&self) -> bool {
        self.selected()
            .is_some_and(|entry| self.expanded_entry_ids.contains(&entry.id))
    }

    pub fn toggle_help(&mut self) {
        self.help_visible = !self.help_visible;
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_line = msg.into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ids::{QueueEntryId, RepoId};
    use time::OffsetDateTime;

    fn entry(status: QueueStatus, branch: &str, enqueued_at: i64) -> QueueEntry {
        QueueEntry {
            id: QueueEntryId::new(),
            repo_id: RepoId::new(),
            source_worktree: "/tmp/x".into(),
            source_branch: branch.into(),
            target_branch: "main".into(),
            status,
            last_outcome: None,
            enqueued_at: OffsetDateTime::from_unix_timestamp(enqueued_at).unwrap(),
            started_at: None,
            finished_at: None,
            failure_reason: None,
            ci_log_dir: None,
            merge_log_path: None,
            conflict_session_id: None,
            message: None,
            details: None,
            claimed_by_pid: None,
            claimed_at: None,
        }
    }

    #[test]
    fn ingest_sorts_by_enqueued_at() {
        let mut app = AppState::new();
        app.ingest_entries(vec![
            entry(QueueStatus::Queued, "feat/c", 30),
            entry(QueueStatus::Queued, "feat/a", 10),
            entry(QueueStatus::Queued, "feat/b", 20),
        ]);
        let branches: Vec<&str> = app
            .entries
            .iter()
            .map(|e| e.source_branch.as_str())
            .collect();
        assert_eq!(branches, vec!["feat/a", "feat/b", "feat/c"]);
    }

    #[test]
    fn ingest_picks_initial_selection() {
        let mut app = AppState::new();
        assert_eq!(app.table.selected(), None);
        app.ingest_entries(vec![entry(QueueStatus::Queued, "feat/a", 1)]);
        assert_eq!(app.table.selected(), Some(0));
    }

    #[test]
    fn ingest_caps_at_max_visible() {
        let mut app = AppState::new();
        let count = MAX_ENTRIES_VISIBLE + 50;
        let many: Vec<_> = (0..count)
            .map(|i| entry(QueueStatus::Queued, "x", i64::try_from(i).unwrap() + 1))
            .collect();
        app.ingest_entries(many);
        assert_eq!(app.entries.len(), MAX_ENTRIES_VISIBLE);
    }

    #[test]
    fn ingest_clamps_selection_when_entries_shrink() {
        let mut app = AppState::new();
        app.ingest_entries(vec![
            entry(QueueStatus::Queued, "a", 1),
            entry(QueueStatus::Queued, "b", 2),
            entry(QueueStatus::Queued, "c", 3),
        ]);
        app.table.select(Some(2)); // last
        app.ingest_entries(vec![entry(QueueStatus::Queued, "a", 1)]);
        assert_eq!(app.table.selected(), Some(0));
    }

    #[test]
    fn ingest_drops_selection_when_emptied() {
        let mut app = AppState::new();
        app.ingest_entries(vec![entry(QueueStatus::Queued, "a", 1)]);
        app.table.select(Some(0));
        app.ingest_entries(Vec::new());
        assert_eq!(app.table.selected(), None);
    }

    #[test]
    fn movement_clamps_at_ends() {
        let mut app = AppState::new();
        app.ingest_entries(vec![
            entry(QueueStatus::Queued, "a", 1),
            entry(QueueStatus::Queued, "b", 2),
        ]);
        app.table.select(Some(0));
        app.move_selection_up();
        assert_eq!(app.table.selected(), Some(0));
        app.move_selection_down();
        assert_eq!(app.table.selected(), Some(1));
        app.move_selection_down();
        assert_eq!(app.table.selected(), Some(1));
    }

    #[test]
    fn deletable_requires_queued() {
        let mut app = AppState::new();
        app.ingest_entries(vec![
            entry(QueueStatus::Queued, "a", 1),
            entry(QueueStatus::Rebasing, "b", 2),
        ]);
        app.table.select(Some(0));
        assert!(app.selected_is_deletable());
        app.table.select(Some(1));
        assert!(!app.selected_is_deletable());
    }

    #[test]
    fn sprite_state_tracks_entries() {
        let mut app = AppState::new();
        assert_eq!(app.sprite_state, SpriteState::Idle);
        app.ingest_entries(vec![entry(QueueStatus::Rebasing, "a", 1)]);
        assert_eq!(app.sprite_state, SpriteState::Working);
        app.ingest_entries(vec![entry(QueueStatus::NeedsHelp, "a", 1)]);
        assert_eq!(app.sprite_state, SpriteState::NeedsHelp);
        app.ingest_entries(Vec::new());
        assert_eq!(app.sprite_state, SpriteState::Idle);
    }

    #[test]
    fn sprite_tick_advances() {
        let mut app = AppState::new();
        let before = app.sprite_tick;
        app.advance_sprite();
        assert_eq!(app.sprite_tick, before + 1);
    }

    #[test]
    fn toggle_selected_expanded_tracks_selected_entry() {
        let mut app = AppState::new();
        app.ingest_entries(vec![entry(QueueStatus::Queued, "a", 1)]);
        app.toggle_selected_expanded();
        assert!(app.selected_is_expanded());
        app.toggle_selected_expanded();
        assert!(!app.selected_is_expanded());
    }

    #[test]
    fn ingest_drops_expanded_ids_for_entries_no_longer_visible() {
        let mut app = AppState::new();
        let a = entry(QueueStatus::Queued, "a", 1);
        let b = entry(QueueStatus::Queued, "b", 2);
        let a_id = a.id;
        let b_id = b.id;
        app.ingest_entries(vec![a, b]);
        app.expanded_entry_ids.insert(a_id);
        app.expanded_entry_ids.insert(b_id);
        app.ingest_entries(vec![entry(QueueStatus::Queued, "c", 3)]);
        assert!(app.expanded_entry_ids.is_empty());
    }
}
