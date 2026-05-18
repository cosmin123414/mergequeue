//! `FakeAgent` + `FakeAgentRegistry` — deterministic `MergeAgent`
//! impls for engine tests.
//!
//! By default a `FakeAgent` records the call and returns an empty
//! `AgentSessionAck`. Construct with `FakeAgent::failing` to make the
//! handoff path fail and exercise the engine's error reporting.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::core::agent_backend::AgentBackend;
use crate::core::ports::{AgentRegistry, AgentSessionAck, ConflictPrompt, MergeAgent, TmuxHandle};
use crate::error::{Error, Result};

#[derive(Debug, Clone)]
pub struct FakeAgentCall {
    pub backend: AgentBackend,
    pub worktree: PathBuf,
    pub prompt: ConflictPrompt,
    pub tmux: TmuxHandle,
}

pub struct FakeAgent {
    backend: AgentBackend,
    fail: bool,
    calls: Mutex<Vec<FakeAgentCall>>,
}

impl FakeAgent {
    pub fn new(backend: AgentBackend) -> Self {
        Self {
            backend,
            fail: false,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn failing(backend: AgentBackend) -> Self {
        Self {
            backend,
            fail: true,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn calls(&self) -> Vec<FakeAgentCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl MergeAgent for FakeAgent {
    fn backend(&self) -> AgentBackend {
        self.backend
    }

    fn check_available(&self) -> Result<()> {
        if self.fail {
            return Err(Error::agent("fake unavailable"));
        }
        Ok(())
    }

    fn open_conflict_session(
        &self,
        worktree: &Path,
        prompt: &ConflictPrompt,
        tmux: &TmuxHandle,
    ) -> Result<AgentSessionAck> {
        self.calls.lock().unwrap().push(FakeAgentCall {
            backend: self.backend,
            worktree: worktree.to_path_buf(),
            prompt: prompt.clone(),
            tmux: tmux.clone(),
        });
        if self.fail {
            return Err(Error::agent("fake handoff failure"));
        }
        Ok(AgentSessionAck {
            agent_session_token: Some(format!("fake-{}", self.backend)),
        })
    }
}

/// Registry that hands out a single `FakeAgent` regardless of backend.
///
/// Tests that want different behavior per backend can construct a
/// custom impl of `AgentRegistry`.
pub struct FakeAgentRegistry {
    agent: Arc<FakeAgent>,
}

impl FakeAgentRegistry {
    pub fn new(agent: Arc<FakeAgent>) -> Self {
        Self { agent }
    }
}

impl AgentRegistry for FakeAgentRegistry {
    fn get(&self, _b: AgentBackend) -> Result<Arc<dyn MergeAgent>> {
        Ok(self.agent.clone())
    }
}
