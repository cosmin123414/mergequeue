//! `MergeAgent` implementations + tmux + prompt rendering.
//!
//! The four `MergeAgent` impls wrap user-installed CLIs (opencode, claude,
//! cursor-agent, codex). MergeSmith never embeds a language model itself.
//! All four shapes are very similar: open a tmux window in the worktree,
//! optionally `send-keys` a prompt into the agent's stdin.

pub mod claude_code;
pub mod codex;
pub mod cursor;
pub mod opencode;
pub mod prompts;
pub mod registry;
pub mod tmux;

pub use registry::DefaultAgentRegistry;
