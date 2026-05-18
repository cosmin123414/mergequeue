//! Feature-gated fakes for the four seam traits.
//!
//! All four impls are deterministic and require no I/O. They are used by
//! engine tests (via `cargo test`) and integration tests (via the
//! `test-support` feature). The `SqliteStore` already supports a
//! `:memory:` mode, so the store fake is optional; we still provide one
//! for the rare test that wants to assert "exactly these store calls in
//! this order."

pub mod fake_agent;
pub mod fake_clock;
pub mod fake_git;
pub mod fake_store;
pub mod fake_tmux;

pub use fake_agent::{FakeAgent, FakeAgentCall, FakeAgentRegistry};
pub use fake_clock::FakeClock;
pub use fake_git::{FakeGit, GitScript};
pub use fake_store::FakeStore;
pub use fake_tmux::{FakeTmux, TmuxCall};
