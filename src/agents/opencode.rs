//! Opencode `MergeAgent` impl.
//!
//! Launch shape:
//!
//! 1. Tmux window already created (engine layer did `new_window`).
//! 2. `send-keys "opencode" Enter` to start the TUI in that window.
//! 3. Wait briefly for the TUI to settle (opencode prints a banner +
//!    waits for input). We use a `tmux send-keys` pause primitive instead
//!    of `sleep` so this stays test-controllable: `wait_for_prompt_ms` is
//!    configurable.
//! 4. `send-keys` the rendered prompt as a single multi-line paste.
//!    Newlines inside the prompt are preserved because we pass the whole
//!    string as one positional argument to `send-keys`.
//! 5. Press `Enter` to submit.
//!
//! After step 5 the agent is alive in the tmux window and the user can
//! attach with `mergesmith resolve <id>`.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::agents::prompts;
use crate::agents::tmux::TmuxOps;
use crate::core::agent_backend::AgentBackend;
use crate::core::ports::{AgentSessionAck, ConflictPrompt, MergeAgent, TmuxHandle};
use crate::error::{Error, Result};

pub struct OpencodeAgent {
    tmux: Arc<dyn TmuxOps>,
    /// Pause between launching `opencode` and pasting the prompt. Real
    /// installs print a small banner; 600ms is comfortable.
    prompt_settle: Duration,
}

impl OpencodeAgent {
    pub fn new(tmux: Arc<dyn TmuxOps>) -> Self {
        Self {
            tmux,
            prompt_settle: Duration::from_millis(600),
        }
    }

    #[cfg(test)]
    pub fn with_settle(tmux: Arc<dyn TmuxOps>, settle: Duration) -> Self {
        Self {
            tmux,
            prompt_settle: settle,
        }
    }
}

impl MergeAgent for OpencodeAgent {
    fn backend(&self) -> AgentBackend {
        AgentBackend::Opencode
    }

    fn check_available(&self) -> Result<()> {
        which::which("opencode")
            .map(|_| ())
            .map_err(|_| Error::agent("`opencode` not found on PATH"))
    }

    fn open_conflict_session(
        &self,
        _worktree: &Path,
        prompt: &ConflictPrompt,
        tmux: &TmuxHandle,
    ) -> Result<AgentSessionAck> {
        let rendered = prompts::render(prompt)?;

        // Launch the TUI in the freshly-created window.
        self.tmux.send_keys(tmux, "opencode", true)?;

        // Wait for the banner to clear before pasting the prompt. In
        // tests with `FakeTmux` and a near-zero settle this is a no-op.
        std::thread::sleep(self.prompt_settle);

        // Send the combined prompt as a single keys argument so that
        // newlines are preserved literally. Trailing `Enter` submits.
        self.tmux.send_keys(tmux, &rendered.combined(), true)?;

        Ok(AgentSessionAck::default())
    }
}
