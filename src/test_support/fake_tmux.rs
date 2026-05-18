//! `FakeTmux` — in-process `TmuxOps` that records calls without ever
//! touching a real tmux server.
//!
//! Tests can assert on call order to verify that the engine's handoff
//! invokes `new_window` and `send_keys` in the expected sequence.

use std::path::Path;
use std::sync::Mutex;

use crate::agents::tmux::TmuxOps;
use crate::core::ports::TmuxHandle;
use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TmuxCall {
    EnsureSession {
        session: String,
    },
    HasSession {
        session: String,
    },
    NewWindow {
        session: String,
        window: String,
        cwd: String,
    },
    SendKeys {
        session: String,
        window: String,
        keys: String,
        enter: bool,
    },
    KillWindow {
        session: String,
        window: String,
    },
    WindowAlive {
        session: String,
        window: String,
    },
}

pub struct FakeTmux {
    state: Mutex<FakeState>,
}

#[derive(Default)]
struct FakeState {
    sessions: Vec<String>,
    windows: Vec<(String, String)>, // (session, window)
    calls: Vec<TmuxCall>,
}

impl Default for FakeTmux {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeTmux {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(FakeState::default()),
        }
    }

    pub fn calls(&self) -> Vec<TmuxCall> {
        self.state.lock().unwrap().calls.clone()
    }

    pub fn windows(&self) -> Vec<(String, String)> {
        self.state.lock().unwrap().windows.clone()
    }

    /// Test hook: forcibly remove a window so `window_alive` returns
    /// `false`. Used to simulate the user killing the tmux window.
    pub fn force_remove_window(&self, handle: &TmuxHandle) {
        let mut g = self.state.lock().unwrap();
        g.windows
            .retain(|(s, w)| !(s == &handle.session && w == &handle.window));
    }
}

impl TmuxOps for FakeTmux {
    fn ensure_session(&self, session: &str) -> Result<()> {
        let mut g = self.state.lock().unwrap();
        g.calls.push(TmuxCall::EnsureSession {
            session: session.to_string(),
        });
        if !g.sessions.iter().any(|s| s == session) {
            g.sessions.push(session.to_string());
        }
        Ok(())
    }

    fn has_session(&self, session: &str) -> Result<bool> {
        let mut g = self.state.lock().unwrap();
        g.calls.push(TmuxCall::HasSession {
            session: session.to_string(),
        });
        Ok(g.sessions.iter().any(|s| s == session))
    }

    fn new_window(&self, session: &str, window: &str, cwd: &Path) -> Result<TmuxHandle> {
        // ensure_session as a side-effect (mirrors ProcessTmux). Use
        // direct mutation to avoid taking the lock twice.
        {
            let mut g = self.state.lock().unwrap();
            if !g.sessions.iter().any(|s| s == session) {
                g.sessions.push(session.to_string());
            }
            g.calls.push(TmuxCall::NewWindow {
                session: session.to_string(),
                window: window.to_string(),
                cwd: cwd.display().to_string(),
            });
            g.windows.push((session.to_string(), window.to_string()));
        }
        Ok(TmuxHandle {
            session: session.to_string(),
            window: window.to_string(),
        })
    }

    fn send_keys(&self, handle: &TmuxHandle, keys: &str, enter: bool) -> Result<()> {
        let mut g = self.state.lock().unwrap();
        g.calls.push(TmuxCall::SendKeys {
            session: handle.session.clone(),
            window: handle.window.clone(),
            keys: keys.to_string(),
            enter,
        });
        Ok(())
    }

    fn kill_window(&self, handle: &TmuxHandle) -> Result<()> {
        let mut g = self.state.lock().unwrap();
        g.calls.push(TmuxCall::KillWindow {
            session: handle.session.clone(),
            window: handle.window.clone(),
        });
        g.windows
            .retain(|(s, w)| !(s == &handle.session && w == &handle.window));
        Ok(())
    }

    fn window_alive(&self, handle: &TmuxHandle) -> Result<bool> {
        let mut g = self.state.lock().unwrap();
        g.calls.push(TmuxCall::WindowAlive {
            session: handle.session.clone(),
            window: handle.window.clone(),
        });
        Ok(g.windows
            .iter()
            .any(|(s, w)| s == &handle.session && w == &handle.window))
    }
}
