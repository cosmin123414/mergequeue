//! `ShutdownToken`: cooperative shutdown signal.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownLevel {
    Running = 0,
    Soft = 1,
    Hard = 2,
}

#[derive(Debug, Clone, Default)]
pub struct ShutdownToken(Arc<AtomicU8>);

impl ShutdownToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn level(&self) -> ShutdownLevel {
        match self.0.load(Ordering::SeqCst) {
            0 => ShutdownLevel::Running,
            1 => ShutdownLevel::Soft,
            _ => ShutdownLevel::Hard,
        }
    }

    pub fn is_set(&self) -> bool {
        self.level() != ShutdownLevel::Running
    }

    pub fn is_hard(&self) -> bool {
        self.level() == ShutdownLevel::Hard
    }

    pub fn set_soft(&self) {
        // Don't downgrade Hard -> Soft.
        let _ = self
            .0
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst);
    }

    pub fn set_hard(&self) {
        self.0.store(2, Ordering::SeqCst);
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
}
