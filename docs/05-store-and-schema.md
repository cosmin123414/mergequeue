# 05 — Store and schema

## Connection

`rusqlite` (bundled SQLite). Pool via `r2d2` so the engine workers can
each hold a handle. Pragmas on every connection:

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous  = NORMAL;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
```

## Migrations

Numbered `.sql` files under `src/store/migrations/` embedded with
`include_str!`. A `schema_version` table tracks applied migrations. On
open: apply any missing migrations in order; refuse to open if DB version
is newer than what the binary knows.

## Initial schema (`0001_init.sql`)

```sql
CREATE TABLE registered_repos (
    id              TEXT PRIMARY KEY,
    root_path       TEXT NOT NULL UNIQUE,
    default_branch  TEXT NOT NULL,
    lint_command    TEXT,
    test_command    TEXT,
    build_command   TEXT,
    dirty_retry_ms  INTEGER NOT NULL,
    agent_backend   TEXT NOT NULL,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE TABLE queue_entries (
    id                   TEXT PRIMARY KEY,
    repo_id              TEXT NOT NULL
                          REFERENCES registered_repos(id) ON DELETE CASCADE,
    source_worktree      TEXT NOT NULL,
    source_branch        TEXT NOT NULL,
    target_branch        TEXT NOT NULL,
    status               TEXT NOT NULL,
    last_outcome         TEXT,
    enqueued_at          INTEGER NOT NULL,
    started_at           INTEGER,
    finished_at          INTEGER,
    failure_reason       TEXT,
    ci_log_dir           TEXT,
    merge_log_path       TEXT,
    conflict_session_id  TEXT REFERENCES conflict_sessions(id),
    message              TEXT,
    claimed_by_pid       INTEGER,
    claimed_at           INTEGER
);

CREATE INDEX idx_queue_repo_status ON queue_entries(repo_id, status, enqueued_at);
CREATE INDEX idx_queue_status      ON queue_entries(status);

CREATE TABLE conflict_sessions (
    id              TEXT PRIMARY KEY,
    queue_entry_id  TEXT NOT NULL REFERENCES queue_entries(id) ON DELETE CASCADE,
    agent_backend   TEXT NOT NULL,
    tmux_session    TEXT NOT NULL,
    tmux_window     TEXT NOT NULL,
    started_at      INTEGER NOT NULL,
    ended_at        INTEGER,
    outcome         TEXT
);

CREATE TABLE schema_version (
    version     INTEGER PRIMARY KEY,
    applied_at  INTEGER NOT NULL
);
```

## Atomic claim

The per-repo FIFO worker uses one `UPDATE … RETURNING …`:

```sql
UPDATE queue_entries
   SET status = 'Rebasing',         -- the FSM's first non-Queued state
       claimed_by_pid = ?pid,
       claimed_at = ?now,
       started_at  = COALESCE(started_at, ?now)
 WHERE id = (
     SELECT id FROM queue_entries
      WHERE repo_id = ?repo
        AND status  = 'Queued'
        AND claimed_by_pid IS NULL
      ORDER BY enqueued_at
      LIMIT 1
 )
RETURNING …;
```

This is the only place `Queued → Rebasing` happens; everywhere else, status
updates flow through `update_entry`.

## Crash recovery

On `SqliteStore::open`, after migrations, the store calls
`sweep_dead_pid_claims(live_pids)`:

```sql
UPDATE queue_entries
   SET status         = 'Queued',
       claimed_by_pid = NULL,
       claimed_at     = NULL,
       last_outcome   = NULL
 WHERE claimed_by_pid IS NOT NULL
   AND claimed_by_pid NOT IN (…live pids…)
   AND status IN ('Rebasing','CIRunning','Merging');
```

`NeedsHelp` entries are not swept — they're a user-action state. On
startup, the engine separately probes any open `conflict_sessions` whose
queue entry is `NeedsHelp`: if the tmux session is gone, mark the conflict
session `Abandoned` (the queue entry stays `NeedsHelp` for the user to
`mergesmith resolve` again).

## PID-file singleton lock for the TUI

`$MERGESMITH_HOME/tui.pid` is created with an exclusive `fs2` flock at
`mergesmith tui` startup. If the file exists and the PID is alive, refuse
the second TUI with a friendly error. If the file exists but the PID is
dead, the lock acquisition will succeed (stale-file handling is automatic
because we use flock, not file existence).
