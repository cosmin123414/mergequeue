//! Cursor `MergeAgent` impl — **M6**. M2 ships a stub.

use std::path::Path;
use std::sync::Arc;

use crate::agents::tmux::TmuxOps;
use crate::core::agent_backend::AgentBackend;
use crate::core::ports::{AgentSessionAck, ConflictPrompt, MergeAgent, TmuxHandle};
use crate::error::{Error, Result};

pub struct CursorAgent {
    #[allow(dead_code)]
    tmux: Arc<dyn TmuxOps>,
}

impl CursorAgent {
    pub fn new(tmux: Arc<dyn TmuxOps>) -> Self {
        Self { tmux }
    }
}

impl MergeAgent for CursorAgent {
    fn backend(&self) -> AgentBackend {
        AgentBackend::Cursor
    }

    fn check_available(&self) -> Result<()> {
        Err(Error::agent("cursor backend lands in M6"))
    }

    fn open_conflict_session(
        &self,
        _worktree: &Path,
        _prompt: &ConflictPrompt,
        _tmux: &TmuxHandle,
    ) -> Result<AgentSessionAck> {
        Err(Error::agent("cursor backend lands in M6"))
    }
}
