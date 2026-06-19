//! `MergeAgent` implementation + tmux + prompt rendering.
//!
//! The `MergeAgent` impl wraps the user-installed `opencode` CLI;
//! MergeQueue never embeds a language model itself. It opens a tmux
//! window in the worktree and `send-keys` a prompt into the agent's
//! stdin.

pub mod opencode;
pub mod prompts;
pub mod registry;
pub mod resolve;
pub mod tmux;

pub use registry::DefaultAgentRegistry;
