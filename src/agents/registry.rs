//! `DefaultAgentRegistry` — the production `AgentRegistry` impl that
//! hands out the `MergeAgent` for a given `AgentBackend`.
//!
//! MergeQueue currently ships a single backend (opencode). The registry
//! seam is kept so additional backends can be added without touching the
//! engine.

use std::sync::Arc;

use crate::agents::opencode::OpencodeAgent;
use crate::agents::tmux::TmuxOps;
use crate::core::agent_backend::AgentBackend;
use crate::core::ports::{AgentRegistry, MergeAgent};
use crate::error::Result;

pub struct DefaultAgentRegistry {
    opencode: Arc<dyn MergeAgent>,
}

impl DefaultAgentRegistry {
    pub fn new(tmux: Arc<dyn TmuxOps>) -> Self {
        Self {
            opencode: Arc::new(OpencodeAgent::new(tmux)),
        }
    }
}

impl AgentRegistry for DefaultAgentRegistry {
    fn get(&self, b: AgentBackend) -> Result<Arc<dyn MergeAgent>> {
        Ok(match b {
            AgentBackend::Opencode => self.opencode.clone(),
        })
    }
}
