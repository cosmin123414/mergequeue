# 04 — The four seam traits

All four live in `src/core/ports.rs` so the engine can import them without
caring which adapter implements them. Each trait is intentionally **narrow**:
it exposes exactly what the engine calls, nothing more.

## `Clock`

```rust
pub trait Clock: Send + Sync {
    fn now(&self) -> OffsetDateTime;
    fn sleep(&self, d: Duration);
}
```

Implementations:

- `store::SystemClock` — wraps `time::OffsetDateTime::now_utc` and
  `std::thread::sleep`.
- `test_support::FakeClock` — virtual clock that advances explicitly via
  `advance(Duration)`; `sleep` returns immediately and records the request.

## `QueueStore`

```rust
pub trait QueueStore: Send + Sync {
    // Repos
    fn list_repos(&self) -> Result<Vec<RegisteredRepo>>;
    fn get_repo(&self, id: RepoId) -> Result<Option<RegisteredRepo>>;
    fn insert_repo(&self, repo: &RegisteredRepo) -> Result<()>;
    fn update_repo(&self, repo: &RegisteredRepo) -> Result<()>;
    fn delete_repo(&self, id: RepoId) -> Result<()>;

    // Queue entries
    fn enqueue(&self, entry: &QueueEntry) -> Result<()>;
    fn list_entries(&self, filter: EntryFilter) -> Result<Vec<QueueEntry>>;
    fn get_entry(&self, id: QueueEntryId) -> Result<Option<QueueEntry>>;
    fn claim_next(&self, repo: RepoId, pid: u32, now: OffsetDateTime)
        -> Result<Option<QueueEntry>>;
    fn update_entry(&self, entry: &QueueEntry) -> Result<()>;
    fn release(&self, id: QueueEntryId) -> Result<()>;
    fn delete_queued(&self, id: QueueEntryId) -> Result<bool>;
    fn sweep_dead_pid_claims(&self, live_pids: &[u32]) -> Result<usize>;

    // Conflict sessions
    fn open_conflict_session(&self, s: &ConflictSession) -> Result<()>;
    fn close_conflict_session(&self, id: ConflictSessionId,
        outcome: ConflictOutcome, when: OffsetDateTime) -> Result<()>;
}
```

Implementations:

- `store::SqliteStore` — real backing store.
- `test_support::FakeStore` — in-memory `Mutex<HashMap>`s, FIFO ordering by
  enqueued_at within each repo.

## `GitOps`

```rust
pub trait GitOps: Send + Sync {
    fn worktree_is_dirty(&self, path: &Path) -> Result<bool>;
    fn current_branch(&self, worktree: &Path) -> Result<String>;
    fn head_sha(&self, worktree: &Path) -> Result<String>;
    fn rebase_onto(&self, worktree: &Path, target_ref: &str)
        -> Result<RebaseOutcome>;
    fn abort_rebase(&self, worktree: &Path) -> Result<()>;
    fn fast_forward(&self, target_worktree: &Path, source_ref: &str)
        -> Result<FastForwardOutcome>;
    fn worktree_exists(&self, path: &Path) -> Result<bool>;
}

pub enum RebaseOutcome { Ok, Conflict, OtherError(String) }
pub enum FastForwardOutcome { Ok, NonFastForward, OtherError(String) }
```

Implementations:

- `git::ProcessGit` — shells out to the user's `git` binary.
- `test_support::FakeGit` — scripted responses (a deque of pre-programmed
  outcomes per method).

## `MergeAgent`

```rust
pub trait MergeAgent: Send + Sync {
    fn open_conflict_session(
        &self,
        worktree: &Path,
        prompt: &ConflictPrompt,
        tmux: &TmuxHandle,
    ) -> Result<ConflictSession>;
}

pub trait AgentRegistry: Send + Sync {
    fn get(&self, b: AgentBackend) -> Result<Arc<dyn MergeAgent>>;
}
```

Implementations:

- `agents::opencode::OpencodeAgent` (the only shipped backend)
- `test_support::FakeAgent` — scripted outcomes.

## Why exactly these four

| Trait | Purpose | Without it |
|---|---|---|
| `Clock` | Test time-based behavior without sleeping | Tests wait real seconds |
| `QueueStore` | Test engine without writing SQLite rows | Tests need tempdir + migrations |
| `GitOps` | Test FSM transitions without real repos | Tests need `git init` + commits per case |
| `MergeAgent` | Test conflict handoff without spawning agents | Tests cost dollars and depend on installed CLIs |

Anything more (e.g. a `CommandRunner` trait around subprocess execution, a
`FileSystem` trait, a `Notifier` trait) is **deliberately not abstracted**.
The CI runner does spawn subprocesses, but it does so through `std::process`
directly — testing the CI runner in isolation isn't valuable enough to
justify another trait.
