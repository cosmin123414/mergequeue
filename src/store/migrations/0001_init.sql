-- Initial schema for MergeSmith.
-- All TEXT IDs are UUIDs serialized as hex with dashes.
-- All timestamps are Unix seconds (INTEGER, UTC).

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

CREATE TABLE conflict_sessions (
    id              TEXT PRIMARY KEY,
    queue_entry_id  TEXT NOT NULL,
    agent_backend   TEXT NOT NULL,
    tmux_session    TEXT NOT NULL,
    tmux_window     TEXT NOT NULL,
    started_at      INTEGER NOT NULL,
    ended_at        INTEGER,
    outcome         TEXT
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
