//! `ShutdownToken`: cooperative shutdown signal.
//!
//! A token holds one or more `Arc<AtomicU8>` sources. `level()` returns
//! the maximum across all sources (`Hard` > `Soft` > `Running`).
//! `set_soft`/`set_hard` write into the *first* source — the one
//! "owned" by this token. Composite tokens (built via
//! [`ShutdownToken::merged`]) thus track all of their parents'
//! escalations while letting the holder still flip their own.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownLevel {
    Running = 0,
    Soft = 1,
    Hard = 2,
}

#[derive(Debug, Clone)]
pub struct ShutdownToken {
    /// The first entry is the token's "own" source — what `set_soft`
    /// and `set_hard` write to. Additional entries are observed-only.
    sources: Vec<Arc<AtomicU8>>,
}

impl Default for ShutdownToken {
    fn default() -> Self {
        Self {
            sources: vec![Arc::new(AtomicU8::new(0))],
        }
    }
}

impl ShutdownToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a token that reports the maximum level across `parents`
    /// AND its own fresh source. Setting this token's level only
    /// affects its own source — the parents remain untouched. Useful
    /// for "this worker is part of a pool; either the pool or the
    /// individual worker can signal shutdown."
    pub fn merged(parents: impl IntoIterator<Item = ShutdownToken>) -> Self {
        let mut sources = vec![Arc::new(AtomicU8::new(0))];
        for parent in parents {
            sources.extend(parent.sources.into_iter());
        }
        Self { sources }
    }

    pub fn level(&self) -> ShutdownLevel {
        let mut max = 0u8;
        for s in &self.sources {
            let v = s.load(Ordering::SeqCst);
            if v > max {
                max = v;
            }
        }
        match max {
            0 => ShutdownLevel::Running,
            1 => ShutdownLevel::Soft,
            _ => ShutdownLevel::Hard,
        }
    }

    pub fn is_set(&self) -> bool {
        self.level() != ShutdownLevel::Running
    }

    /// Set this token's own source to `Soft`. No-op if it has already
    /// escalated to `Hard`.
    pub fn set_soft(&self) {
        let own = &self.sources[0];
        let _ = own.compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst);
    }

    /// Set this token's own source to `Hard`. Always succeeds.
    pub fn set_hard(&self) {
        self.sources[0].store(2, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_only_escalate() {
        let t = ShutdownToken::new();
        assert_eq!(t.level(), ShutdownLevel::Running);
        t.set_soft();
        assert_eq!(t.level(), ShutdownLevel::Soft);
        t.set_hard();
        assert_eq!(t.level(), ShutdownLevel::Hard);
        // Once hard, set_soft is a no-op.
        t.set_soft();
        assert_eq!(t.level(), ShutdownLevel::Hard);
    }

    #[test]
    fn merged_observes_parent_escalation() {
        let parent = ShutdownToken::new();
        let child = ShutdownToken::merged([parent.clone()]);
        assert!(!child.is_set());

        parent.set_soft();
        assert_eq!(child.level(), ShutdownLevel::Soft);
        assert_eq!(parent.level(), ShutdownLevel::Soft);

        // Hard on the parent shows through the child.
        parent.set_hard();
        assert_eq!(child.level(), ShutdownLevel::Hard);
    }

    #[test]
    fn merged_set_does_not_affect_parent() {
        let parent = ShutdownToken::new();
        let child = ShutdownToken::merged([parent.clone()]);

        child.set_soft();
        assert_eq!(child.level(), ShutdownLevel::Soft);
        assert_eq!(parent.level(), ShutdownLevel::Running, "parent untouched");
    }

    #[test]
    fn merged_reports_max_of_multiple_parents() {
        let a = ShutdownToken::new();
        let b = ShutdownToken::new();
        let c = ShutdownToken::merged([a.clone(), b.clone()]);
        b.set_hard();
        assert_eq!(c.level(), ShutdownLevel::Hard);
    }
}
