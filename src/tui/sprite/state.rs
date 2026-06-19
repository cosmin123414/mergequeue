//! Sprite state machine.
//!
//! Maps a snapshot of the queue (taken once per render tick) to a
//! [`SpriteState`]. Three states for M4:
//!
//! - `Idle` — nothing is in flight.
//! - `Working` — at least one entry is being processed (`Rebasing`,
//!   `CIRunning`, or `Merging`).
//! - `NeedsHelp` — at least one entry is awaiting human/agent
//!   intervention.
//!
//! Priority `NeedsHelp > Working > Idle`. This is the user-attention
//! ordering: if anything needs the user, that's what the animation
//! should be signalling, even if other entries are humming along.
//!
//! The chosen state tunes the Penrose animation (zoom speed + accent
//! color) rather than selecting a sprite-sheet frame.

use crate::core::queue::{QueueEntry, QueueStatus};

/// A snapshot of the queue, computed once per render tick.
///
/// Carrying just the status counts (not the full entries) keeps the
/// hot path cheap and makes the sprite state derivation a pure
/// function of three integers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QueueSnapshot {
    pub queued: u32,
    pub rebasing: u32,
    pub ci_running: u32,
    pub merging: u32,
    pub needs_help: u32,
    pub merged: u32,
    pub failed: u32,
    pub cancelled: u32,
}

impl QueueSnapshot {
    /// Build a snapshot from a slice of `QueueEntry`. Cheap; O(n) in
    /// the slice size and we only ever pass a few dozen entries.
    pub fn from_entries(entries: &[QueueEntry]) -> Self {
        let mut s = Self::default();
        for e in entries {
            match e.status {
                QueueStatus::Queued => s.queued += 1,
                QueueStatus::Rebasing => s.rebasing += 1,
                QueueStatus::CIRunning => s.ci_running += 1,
                QueueStatus::Merging => s.merging += 1,
                QueueStatus::NeedsHelp => s.needs_help += 1,
                QueueStatus::Merged => s.merged += 1,
                QueueStatus::Failed => s.failed += 1,
                QueueStatus::Cancelled => s.cancelled += 1,
            }
        }
        s
    }

    pub fn total_active(&self) -> u32 {
        self.queued + self.rebasing + self.ci_running + self.merging + self.needs_help
    }
}

/// Which mood the Penrose animation should render this tick.
///
/// The variant tunes the animator's zoom speed and accent color (see
/// [`crate::tui::sprite::penrose::PenroseAnimator::frame`]); the tiling
/// geometry is identical across states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpriteState {
    Idle,
    Working,
    NeedsHelp,
}

/// Pick the sprite state from a queue snapshot. Pure function.
pub fn compute_sprite_state(snap: &QueueSnapshot) -> SpriteState {
    if snap.needs_help > 0 {
        SpriteState::NeedsHelp
    } else if snap.rebasing + snap.ci_running + snap.merging > 0 {
        SpriteState::Working
    } else {
        SpriteState::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ids::{QueueEntryId, RepoId};
    use crate::core::queue::QueueEntry;
    use time::OffsetDateTime;

    fn entry(status: QueueStatus) -> QueueEntry {
        QueueEntry {
            id: QueueEntryId::new(),
            repo_id: RepoId::new(),
            source_worktree: "/tmp/x".into(),
            source_branch: "feat/x".into(),
            target_branch: "main".into(),
            status,
            last_outcome: None,
            enqueued_at: OffsetDateTime::from_unix_timestamp(0).unwrap(),
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
    fn empty_queue_is_idle() {
        let snap = QueueSnapshot::default();
        assert_eq!(compute_sprite_state(&snap), SpriteState::Idle);
    }

    #[test]
    fn only_queued_is_idle() {
        // Queued entries don't count as work-in-flight; the animation
        // stays Idle until something is actually claimed and processing.
        let snap = QueueSnapshot::from_entries(&[entry(QueueStatus::Queued)]);
        assert_eq!(compute_sprite_state(&snap), SpriteState::Idle);
    }

    #[test]
    fn rebasing_is_working() {
        let snap = QueueSnapshot::from_entries(&[entry(QueueStatus::Rebasing)]);
        assert_eq!(compute_sprite_state(&snap), SpriteState::Working);
    }

    #[test]
    fn ci_running_is_working() {
        let snap = QueueSnapshot::from_entries(&[entry(QueueStatus::CIRunning)]);
        assert_eq!(compute_sprite_state(&snap), SpriteState::Working);
    }

    #[test]
    fn merging_is_working() {
        let snap = QueueSnapshot::from_entries(&[entry(QueueStatus::Merging)]);
        assert_eq!(compute_sprite_state(&snap), SpriteState::Working);
    }

    #[test]
    fn needs_help_dominates_working() {
        let snap = QueueSnapshot::from_entries(&[
            entry(QueueStatus::Rebasing),
            entry(QueueStatus::NeedsHelp),
        ]);
        assert_eq!(compute_sprite_state(&snap), SpriteState::NeedsHelp);
    }

    #[test]
    fn terminal_statuses_alone_are_idle() {
        let snap = QueueSnapshot::from_entries(&[
            entry(QueueStatus::Merged),
            entry(QueueStatus::Failed),
            entry(QueueStatus::Cancelled),
        ]);
        assert_eq!(compute_sprite_state(&snap), SpriteState::Idle);
    }

    #[test]
    fn snapshot_counts_each_status_once() {
        let entries: Vec<_> = [
            QueueStatus::Queued,
            QueueStatus::Queued,
            QueueStatus::Rebasing,
            QueueStatus::Merged,
            QueueStatus::Cancelled,
            QueueStatus::Failed,
            QueueStatus::NeedsHelp,
        ]
        .iter()
        .map(|s| entry(*s))
        .collect();
        let snap = QueueSnapshot::from_entries(&entries);
        assert_eq!(snap.queued, 2);
        assert_eq!(snap.rebasing, 1);
        assert_eq!(snap.merged, 1);
        assert_eq!(snap.cancelled, 1);
        assert_eq!(snap.failed, 1);
        assert_eq!(snap.needs_help, 1);
        assert_eq!(snap.total_active(), 4); // queued + rebasing + needs_help
    }
}
