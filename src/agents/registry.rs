//! `DefaultAgentRegistry` — the production `AgentRegistry` impl that
//! hands out the right `MergeAgent` for a given `AgentBackend`.
//!
//! Instances are constructed eagerly at startup (cheap; each backend is
//! a zero-sized struct holding only an `Arc<dyn TmuxOps>`). On `get()`
//! we clone the matching `Arc`.

use std::sync::Arc;

use crate::agents::claude_code::ClaudeCodeAgent;
use crate::agents::codex::CodexAgent;
use crate::agents::cursor::CursorAgent;
use crate::agents::opencode::OpencodeAgent;
use crate::agents::tmux::TmuxOps;
use crate::core::agent_backend::AgentBackend;
use crate::core::ports::{AgentRegistry, MergeAgent};
use crate::error::Result;

pub struct DefaultAgentRegistry {
    opencode: Arc<dyn MergeAgent>,
    claude_code: Arc<dyn MergeAgent>,
    cursor: Arc<dyn MergeAgent>,
    codex: Arc<dyn MergeAgent>,
}

impl DefaultAgentRegistry {
    pub fn new(tmux: Arc<dyn TmuxOps>) -> Self {
        Self {
            opencode: Arc::new(OpencodeAgent::new(tmux.clone())),
            claude_code: Arc::new(ClaudeCodeAgent::new(tmux.clone())),
            cursor: Arc::new(CursorAgent::new(tmux.clone())),
            codex: Arc::new(CodexAgent::new(tmux)),
        }
    }
}

impl AgentRegistry for DefaultAgentRegistry {
    fn get(&self, b: AgentBackend) -> Result<Arc<dyn MergeAgent>> {
        Ok(match b {
            AgentBackend::Opencode => self.opencode.clone(),
            AgentBackend::ClaudeCode => self.claude_code.clone(),
            AgentBackend::Cursor => self.cursor.clone(),
            AgentBackend::Codex => self.codex.clone(),
        })
    }
}
