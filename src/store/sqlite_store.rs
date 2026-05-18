//! `SqliteStore`: the production `QueueStore` impl.
//!
//! All queries are written by hand; we do not pull in a query builder.
//! Each row gets a single `from_row` mapping. The store is `Send + Sync`
//! by wrapping the `Connection` in a `Mutex` — engine workers hold it
//! briefly per call. SQLite WAL mode allows concurrent CLI processes to
//! enqueue while a TUI's workers are reading.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Mutex;
use std::time::Duration;

use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row};
use time::OffsetDateTime;

use crate::core::agent_backend::AgentBackend;
use crate::core::conflict::{ConflictOutcome, ConflictSession};
use crate::core::ids::{ConflictSessionId, QueueEntryId, RepoId};
use crate::core::ports::{EntryFilter, QueueStore};
use crate::core::queue::{MergeFailureReason, QueueEntry, QueueStatus, StepOutcome};
use crate::core::repo::{RegisteredRepo, RepoCiConfig};
use crate::error::{Error, Result};
use crate::store::{connection, migrations};

pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    /// Open (and migrate) the store at `path`. The caller is responsible
    /// for ensuring the parent directory exists.
    pub fn open(path: &Path) -> Result<Self> {
        let mut conn = connection::open_with_pragmas(path)?;
        migrations::run(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// In-memory store for fast tests against the real schema.
    pub fn open_in_memory() -> Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        connection::apply_pragmas(&conn)?;
        migrations::run(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn with_conn<R>(&self, f: impl FnOnce(&Connection) -> Result<R>) -> Result<R> {
        let conn = self.conn.lock().expect("SqliteStore mutex poisoned");
        f(&conn)
    }
}

// ---------------------------------------------------------------------
// Row mapping
// ---------------------------------------------------------------------

fn ts_to_dt(secs: i64) -> Result<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp(secs)
        .map_err(|e| Error::other(format!("invalid timestamp {secs}: {e}")))
}

fn dt_to_ts(dt: OffsetDateTime) -> i64 {
    dt.unix_timestamp()
}

fn opt_dt_to_ts(dt: Option<OffsetDateTime>) -> Option<i64> {
    dt.map(dt_to_ts)
}

fn opt_ts_to_dt(ts: Option<i64>) -> Result<Option<OffsetDateTime>> {
    ts.map(ts_to_dt).transpose()
}

fn parse_status(s: &str) -> Result<QueueStatus> {
    match s {
        "Queued" => Ok(QueueStatus::Queued),
        "Rebasing" => Ok(QueueStatus::Rebasing),
        "CIRunning" => Ok(QueueStatus::CIRunning),
        "Merging" => Ok(QueueStatus::Merging),
        "NeedsHelp" => Ok(QueueStatus::NeedsHelp),
        "Merged" => Ok(QueueStatus::Merged),
        "Failed" => Ok(QueueStatus::Failed),
        "Cancelled" => Ok(QueueStatus::Cancelled),
        other => Err(Error::other(format!("unknown status {other}"))),
    }
}

fn parse_outcome(s: &str) -> Result<StepOutcome> {
    match s {
        "PrecheckOk" => Ok(StepOutcome::PrecheckOk),
        "PrecheckDirtyTarget" => Ok(StepOutcome::PrecheckDirtyTarget),
        "RebaseOk" => Ok(StepOutcome::RebaseOk),
        "RebaseConflict" => Ok(StepOutcome::RebaseConflict),
        "LintPassed" => Ok(StepOutcome::LintPassed),
        "LintFailed" => Ok(StepOutcome::LintFailed),
        "TestPassed" => Ok(StepOutcome::TestPassed),
        "TestFailed" => Ok(StepOutcome::TestFailed),
        "BuildPassed" => Ok(StepOutcome::BuildPassed),
        "BuildFailed" => Ok(StepOutcome::BuildFailed),
        "FastForwardOk" => Ok(StepOutcome::FastForwardOk),
        "FastForwardRejected" => Ok(StepOutcome::FastForwardRejected),
        "AgentResolvedConflict" => Ok(StepOutcome::AgentResolvedConflict),
        "UserCancelled" => Ok(StepOutcome::UserCancelled),
        other => Err(Error::other(format!("unknown outcome {other}"))),
    }
}

fn parse_failure_reason(s: &str) -> Result<MergeFailureReason> {
    match s {
        "TargetWorktreeDirty" => Ok(MergeFailureReason::TargetWorktreeDirty),
        "RebaseUnresolvable" => Ok(MergeFailureReason::RebaseUnresolvable),
        "CILintFailed" => Ok(MergeFailureReason::CILintFailed),
        "CITestsFailed" => Ok(MergeFailureReason::CITestsFailed),
        "CIBuildFailed" => Ok(MergeFailureReason::CIBuildFailed),
        "FastForwardFailed" => Ok(MergeFailureReason::FastForwardFailed),
        "WorktreeGone" => Ok(MergeFailureReason::WorktreeGone),
        "UserCancelled" => Ok(MergeFailureReason::UserCancelled),
        "UncleanShutdown" => Ok(MergeFailureReason::UncleanShutdown),
        other => Err(Error::other(format!("unknown failure reason {other}"))),
    }
}

#[cfg(test)]
fn parse_conflict_outcome(s: &str) -> Result<ConflictOutcome> {
    match s {
        "Resolved" => Ok(ConflictOutcome::Resolved),
        "Abandoned" => Ok(ConflictOutcome::Abandoned),
        "AgentCrashed" => Ok(ConflictOutcome::AgentCrashed),
        other => Err(Error::other(format!("unknown conflict outcome {other}"))),
    }
}

fn repo_from_row(row: &Row<'_>) -> rusqlite::Result<RegisteredRepo> {
    let id_str: String = row.get("id")?;
    let id = RepoId::from_str(&id_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let root_path: String = row.get("root_path")?;
    let default_branch: String = row.get("default_branch")?;
    let lint_command: Option<String> = row.get("lint_command")?;
    let test_command: Option<String> = row.get("test_command")?;
    let build_command: Option<String> = row.get("build_command")?;
    let dirty_retry_ms: i64 = row.get("dirty_retry_ms")?;
    let agent_backend: String = row.get("agent_backend")?;
    let created_at: i64 = row.get("created_at")?;
    let updated_at: i64 = row.get("updated_at")?;

    Ok(RegisteredRepo {
        id,
        root_path: PathBuf::from(root_path),
        default_branch,
        ci: RepoCiConfig {
            lint_command,
            test_command,
            build_command,
            dirty_retry: Duration::from_millis(u64::try_from(dirty_retry_ms).unwrap_or(0)),
        },
        agent_backend: AgentBackend::from_str(&agent_backend).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?,
        created_at: ts_to_dt(created_at).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Integer,
                Box::new(e),
            )
        })?,
        updated_at: ts_to_dt(updated_at).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Integer,
                Box::new(e),
            )
        })?,
    })
}

fn entry_from_row(row: &Row<'_>) -> rusqlite::Result<QueueEntry> {
    let id_str: String = row.get("id")?;
    let id = QueueEntryId::from_str(&id_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let repo_id_str: String = row.get("repo_id")?;
    let repo_id = RepoId::from_str(&repo_id_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let source_worktree: String = row.get("source_worktree")?;
    let source_branch: String = row.get("source_branch")?;
    let target_branch: String = row.get("target_branch")?;
    let status: String = row.get("status")?;
    let last_outcome: Option<String> = row.get("last_outcome")?;
    let enqueued_at: i64 = row.get("enqueued_at")?;
    let started_at: Option<i64> = row.get("started_at")?;
    let finished_at: Option<i64> = row.get("finished_at")?;
    let failure_reason: Option<String> = row.get("failure_reason")?;
    let ci_log_dir: Option<String> = row.get("ci_log_dir")?;
    let merge_log_path: Option<String> = row.get("merge_log_path")?;
    let conflict_session_id_str: Option<String> = row.get("conflict_session_id")?;
    let message: Option<String> = row.get("message")?;
    let claimed_by_pid: Option<i64> = row.get("claimed_by_pid")?;
    let claimed_at: Option<i64> = row.get("claimed_at")?;

    let conv = |e: Error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };

    Ok(QueueEntry {
        id,
        repo_id,
        source_worktree: PathBuf::from(source_worktree),
        source_branch,
        target_branch,
        status: parse_status(&status).map_err(conv)?,
        last_outcome: last_outcome
            .map(|s| parse_outcome(&s))
            .transpose()
            .map_err(conv)?,
        enqueued_at: ts_to_dt(enqueued_at).map_err(conv)?,
        started_at: opt_ts_to_dt(started_at).map_err(conv)?,
        finished_at: opt_ts_to_dt(finished_at).map_err(conv)?,
        failure_reason: failure_reason
            .map(|s| parse_failure_reason(&s))
            .transpose()
            .map_err(conv)?,
        ci_log_dir: ci_log_dir.map(PathBuf::from),
        merge_log_path: merge_log_path.map(PathBuf::from),
        conflict_session_id: conflict_session_id_str
            .map(|s| ConflictSessionId::from_str(&s))
            .transpose()
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?,
        message,
        claimed_by_pid: claimed_by_pid.map(|v| u32::try_from(v).unwrap_or(0)),
        claimed_at: opt_ts_to_dt(claimed_at).map_err(conv)?,
    })
}

#[cfg(test)]
fn conflict_from_row(row: &Row<'_>) -> rusqlite::Result<ConflictSession> {
    let conv_id = |e: uuid::Error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    let conv_str = |e: Error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    let id_str: String = row.get("id")?;
    let qid_str: String = row.get("queue_entry_id")?;
    let backend: String = row.get("agent_backend")?;
    let started_at: i64 = row.get("started_at")?;
    let ended_at: Option<i64> = row.get("ended_at")?;
    let outcome: Option<String> = row.get("outcome")?;

    Ok(ConflictSession {
        id: ConflictSessionId::from_str(&id_str).map_err(conv_id)?,
        queue_entry_id: QueueEntryId::from_str(&qid_str).map_err(conv_id)?,
        agent_backend: AgentBackend::from_str(&backend).map_err(conv_str)?,
        tmux_session: row.get("tmux_session")?,
        tmux_window: row.get("tmux_window")?,
        started_at: ts_to_dt(started_at).map_err(conv_str)?,
        ended_at: opt_ts_to_dt(ended_at).map_err(conv_str)?,
        outcome: outcome
            .map(|s| parse_conflict_outcome(&s))
            .transpose()
            .map_err(conv_str)?,
    })
}

// ---------------------------------------------------------------------
// QueueStore impl
// ---------------------------------------------------------------------

impl QueueStore for SqliteStore {
    fn list_repos(&self) -> Result<Vec<RegisteredRepo>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare("SELECT * FROM registered_repos ORDER BY created_at")?;
            let rows = stmt.query_map([], repo_from_row)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    fn get_repo(&self, id: RepoId) -> Result<Option<RegisteredRepo>> {
        self.with_conn(|conn| {
            Ok(conn
                .query_row(
                    "SELECT * FROM registered_repos WHERE id = ?1",
                    [id.to_string()],
                    repo_from_row,
                )
                .optional()?)
        })
    }

    fn get_repo_by_root(&self, root: &Path) -> Result<Option<RegisteredRepo>> {
        let root_s = root.to_string_lossy().to_string();
        self.with_conn(|conn| {
            Ok(conn
                .query_row(
                    "SELECT * FROM registered_repos WHERE root_path = ?1",
                    [root_s],
                    repo_from_row,
                )
                .optional()?)
        })
    }

    fn insert_repo(&self, repo: &RegisteredRepo) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO registered_repos (id, root_path, default_branch,
                 lint_command, test_command, build_command, dirty_retry_ms,
                 agent_backend, created_at, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    repo.id.to_string(),
                    repo.root_path.to_string_lossy(),
                    repo.default_branch,
                    repo.ci.lint_command,
                    repo.ci.test_command,
                    repo.ci.build_command,
                    i64::try_from(repo.ci.dirty_retry.as_millis()).unwrap_or(i64::MAX),
                    repo.agent_backend.as_str(),
                    dt_to_ts(repo.created_at),
                    dt_to_ts(repo.updated_at),
                ],
            )?;
            Ok(())
        })
    }

    fn update_repo(&self, repo: &RegisteredRepo) -> Result<()> {
        let updated = self.with_conn(|conn| {
            Ok(conn.execute(
                "UPDATE registered_repos
                   SET root_path = ?2,
                       default_branch = ?3,
                       lint_command = ?4,
                       test_command = ?5,
                       build_command = ?6,
                       dirty_retry_ms = ?7,
                       agent_backend = ?8,
                       updated_at = ?9
                 WHERE id = ?1",
                params![
                    repo.id.to_string(),
                    repo.root_path.to_string_lossy(),
                    repo.default_branch,
                    repo.ci.lint_command,
                    repo.ci.test_command,
                    repo.ci.build_command,
                    i64::try_from(repo.ci.dirty_retry.as_millis()).unwrap_or(i64::MAX),
                    repo.agent_backend.as_str(),
                    dt_to_ts(repo.updated_at),
                ],
            )?)
        })?;
        if updated == 0 {
            return Err(Error::NotFound(format!("repo {}", repo.id)));
        }
        Ok(())
    }

    fn delete_repo(&self, id: RepoId) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "DELETE FROM registered_repos WHERE id = ?1",
                [id.to_string()],
            )?;
            Ok(())
        })
    }

    fn enqueue(&self, entry: &QueueEntry) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO queue_entries (
                    id, repo_id, source_worktree, source_branch, target_branch,
                    status, last_outcome, enqueued_at, started_at, finished_at,
                    failure_reason, ci_log_dir, merge_log_path,
                    conflict_session_id, message, claimed_by_pid, claimed_at
                 ) VALUES (
                    ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17
                 )",
                params![
                    entry.id.to_string(),
                    entry.repo_id.to_string(),
                    entry.source_worktree.to_string_lossy(),
                    entry.source_branch,
                    entry.target_branch,
                    entry.status.as_str(),
                    entry.last_outcome.map(|o| o.as_str()),
                    dt_to_ts(entry.enqueued_at),
                    opt_dt_to_ts(entry.started_at),
                    opt_dt_to_ts(entry.finished_at),
                    entry.failure_reason.map(|r| r.as_str()),
                    entry
                        .ci_log_dir
                        .as_ref()
                        .map(|p| p.to_string_lossy().to_string()),
                    entry
                        .merge_log_path
                        .as_ref()
                        .map(|p| p.to_string_lossy().to_string()),
                    entry.conflict_session_id.map(|c| c.to_string()),
                    entry.message,
                    entry.claimed_by_pid.map(i64::from),
                    opt_dt_to_ts(entry.claimed_at),
                ],
            )?;
            Ok(())
        })
    }

    fn list_entries(&self, filter: EntryFilter) -> Result<Vec<QueueEntry>> {
        let mut sql = String::from("SELECT * FROM queue_entries");
        let mut clauses: Vec<String> = Vec::new();
        let mut binds: Vec<String> = Vec::new();

        if let Some(repo) = filter.repo_id {
            clauses.push(format!("repo_id = ?{}", binds.len() + 1));
            binds.push(repo.to_string());
        }
        if let Some(statuses) = filter.statuses {
            if !statuses.is_empty() {
                let placeholders: Vec<String> = (0..statuses.len())
                    .map(|i| format!("?{}", binds.len() + 1 + i))
                    .collect();
                clauses.push(format!("status IN ({})", placeholders.join(",")));
                for s in statuses {
                    binds.push(s.as_str().to_string());
                }
            }
        }
        if !clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&clauses.join(" AND "));
        }
        sql.push_str(" ORDER BY enqueued_at");

        self.with_conn(|conn| {
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params_from_iter(binds.iter()), entry_from_row)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    fn get_entry(&self, id: QueueEntryId) -> Result<Option<QueueEntry>> {
        self.with_conn(|conn| {
            Ok(conn
                .query_row(
                    "SELECT * FROM queue_entries WHERE id = ?1",
                    [id.to_string()],
                    entry_from_row,
                )
                .optional()?)
        })
    }

    fn claim_next(
        &self,
        repo: RepoId,
        pid: u32,
        now: OffsetDateTime,
    ) -> Result<Option<QueueEntry>> {
        // Atomic claim: pick the oldest Queued entry for this repo with no
        // active claim, flip it to Rebasing, stamp claim metadata. The
        // FSM's first action after a successful claim is `Rebase`, so the
        // status flip here matches that.
        let now_ts = dt_to_ts(now);
        self.with_conn(|conn| {
            // SQLite supports `UPDATE … RETURNING …` since 3.35.
            let mut stmt = conn.prepare(
                "UPDATE queue_entries
                   SET status         = 'Rebasing',
                       claimed_by_pid = ?2,
                       claimed_at     = ?3,
                       started_at     = COALESCE(started_at, ?3),
                       last_outcome   = 'PrecheckOk'
                 WHERE id = (
                     SELECT id FROM queue_entries
                      WHERE repo_id = ?1
                        AND status  = 'Queued'
                        AND claimed_by_pid IS NULL
                      ORDER BY enqueued_at
                      LIMIT 1
                 )
                RETURNING *",
            )?;
            let row = stmt
                .query_row(
                    params![repo.to_string(), i64::from(pid), now_ts],
                    entry_from_row,
                )
                .optional()?;
            Ok(row)
        })
    }

    fn update_entry(&self, entry: &QueueEntry) -> Result<()> {
        let updated = self.with_conn(|conn| {
            Ok(conn.execute(
                "UPDATE queue_entries
                   SET status              = ?2,
                       last_outcome        = ?3,
                       started_at          = ?4,
                       finished_at         = ?5,
                       failure_reason      = ?6,
                       ci_log_dir          = ?7,
                       merge_log_path      = ?8,
                       conflict_session_id = ?9,
                       message             = ?10,
                       claimed_by_pid      = ?11,
                       claimed_at          = ?12
                 WHERE id = ?1",
                params![
                    entry.id.to_string(),
                    entry.status.as_str(),
                    entry.last_outcome.map(|o| o.as_str()),
                    opt_dt_to_ts(entry.started_at),
                    opt_dt_to_ts(entry.finished_at),
                    entry.failure_reason.map(|r| r.as_str()),
                    entry
                        .ci_log_dir
                        .as_ref()
                        .map(|p| p.to_string_lossy().to_string()),
                    entry
                        .merge_log_path
                        .as_ref()
                        .map(|p| p.to_string_lossy().to_string()),
                    entry.conflict_session_id.map(|c| c.to_string()),
                    entry.message,
                    entry.claimed_by_pid.map(i64::from),
                    opt_dt_to_ts(entry.claimed_at),
                ],
            )?)
        })?;
        if updated == 0 {
            return Err(Error::NotFound(format!("entry {}", entry.id)));
        }
        Ok(())
    }

    fn release(&self, id: QueueEntryId) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE queue_entries
                   SET status = 'Queued',
                       claimed_by_pid = NULL,
                       claimed_at = NULL,
                       last_outcome = NULL
                 WHERE id = ?1
                   AND status IN ('Rebasing','CIRunning','Merging')",
                [id.to_string()],
            )?;
            Ok(())
        })
    }

    fn delete_queued(&self, id: QueueEntryId) -> Result<bool> {
        self.with_conn(|conn| {
            let n = conn.execute(
                "DELETE FROM queue_entries WHERE id = ?1 AND status = 'Queued'",
                [id.to_string()],
            )?;
            Ok(n > 0)
        })
    }

    fn sweep_dead_pid_claims(&self, live_pids: &[u32]) -> Result<usize> {
        // Build `NOT IN (…)` dynamically; SQLite has no array type.
        let mut sql = String::from(
            "UPDATE queue_entries
               SET status = 'Queued',
                   claimed_by_pid = NULL,
                   claimed_at = NULL,
                   last_outcome = NULL
             WHERE status IN ('Rebasing','CIRunning','Merging')
               AND claimed_by_pid IS NOT NULL",
        );
        let binds: Vec<i64> = live_pids.iter().map(|p| i64::from(*p)).collect();
        if !binds.is_empty() {
            use std::fmt::Write;
            let placeholders: Vec<String> =
                (0..binds.len()).map(|i| format!("?{}", i + 1)).collect();
            write!(
                &mut sql,
                " AND claimed_by_pid NOT IN ({})",
                placeholders.join(",")
            )
            .expect("write into String never fails");
        }
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(&sql)?;
            let n = stmt.execute(params_from_iter(binds.iter()))?;
            Ok(n)
        })
    }

    fn open_conflict_session(&self, s: &ConflictSession) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO conflict_sessions (
                    id, queue_entry_id, agent_backend, tmux_session, tmux_window,
                    started_at, ended_at, outcome
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    s.id.to_string(),
                    s.queue_entry_id.to_string(),
                    s.agent_backend.as_str(),
                    s.tmux_session,
                    s.tmux_window,
                    dt_to_ts(s.started_at),
                    opt_dt_to_ts(s.ended_at),
                    s.outcome.map(|o| o.as_str()),
                ],
            )?;
            Ok(())
        })
    }

    fn close_conflict_session(
        &self,
        id: ConflictSessionId,
        outcome: ConflictOutcome,
        when: OffsetDateTime,
    ) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE conflict_sessions
                   SET outcome = ?2, ended_at = ?3
                 WHERE id = ?1",
                params![id.to_string(), outcome.as_str(), dt_to_ts(when)],
            )?;
            Ok(())
        })
    }
}

#[cfg(test)]
impl SqliteStore {
    /// Test helper to read back a conflict session.
    pub fn get_conflict_session(&self, id: ConflictSessionId) -> Result<Option<ConflictSession>> {
        self.with_conn(|conn| {
            Ok(conn
                .query_row(
                    "SELECT * FROM conflict_sessions WHERE id = ?1",
                    [id.to_string()],
                    conflict_from_row,
                )
                .optional()?)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use time::OffsetDateTime;

    fn now() -> OffsetDateTime {
        // Truncate to second-precision so round-trips compare equal
        // (the store rounds to unix_timestamp() seconds).
        OffsetDateTime::from_unix_timestamp(OffsetDateTime::now_utc().unix_timestamp()).unwrap()
    }

    fn sample_repo() -> RegisteredRepo {
        RegisteredRepo {
            id: RepoId::new(),
            root_path: PathBuf::from("/tmp/sample-repo"),
            default_branch: "main".into(),
            ci: RepoCiConfig::default(),
            agent_backend: AgentBackend::Opencode,
            created_at: now(),
            updated_at: now(),
        }
    }

    fn sample_entry(repo: RepoId) -> QueueEntry {
        QueueEntry {
            id: QueueEntryId::new(),
            repo_id: repo,
            source_worktree: PathBuf::from("/tmp/sample-repo/feat"),
            source_branch: "feat/x".into(),
            target_branch: "main".into(),
            status: QueueStatus::Queued,
            last_outcome: None,
            enqueued_at: now(),
            started_at: None,
            finished_at: None,
            failure_reason: None,
            ci_log_dir: None,
            merge_log_path: None,
            conflict_session_id: None,
            message: None,
            claimed_by_pid: None,
            claimed_at: None,
        }
    }

    #[test]
    fn insert_and_list_repo() {
        let s = SqliteStore::open_in_memory().unwrap();
        let r = sample_repo();
        s.insert_repo(&r).unwrap();
        let list = s.list_repos().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].root_path, r.root_path);
    }

    #[test]
    fn enqueue_then_get_round_trip() {
        let s = SqliteStore::open_in_memory().unwrap();
        let r = sample_repo();
        s.insert_repo(&r).unwrap();
        let e = sample_entry(r.id);
        s.enqueue(&e).unwrap();
        let fetched = s.get_entry(e.id).unwrap().unwrap();
        assert_eq!(fetched.id, e.id);
        assert_eq!(fetched.source_branch, "feat/x");
    }

    #[test]
    fn claim_next_returns_oldest_queued_only_once() {
        let s = SqliteStore::open_in_memory().unwrap();
        let r = sample_repo();
        s.insert_repo(&r).unwrap();
        let e1 = sample_entry(r.id);
        std::thread::sleep(std::time::Duration::from_millis(10));
        let mut e2 = sample_entry(r.id);
        // ensure e2 enqueued strictly after e1
        e2.enqueued_at = e1.enqueued_at + time::Duration::seconds(1);
        s.enqueue(&e1).unwrap();
        s.enqueue(&e2).unwrap();

        let claimed = s.claim_next(r.id, 42, now()).unwrap().unwrap();
        assert_eq!(claimed.id, e1.id);
        assert_eq!(claimed.status, QueueStatus::Rebasing);
        assert_eq!(claimed.claimed_by_pid, Some(42));

        // Second claim returns e2 (e1 is no longer Queued).
        let claimed2 = s.claim_next(r.id, 42, now()).unwrap().unwrap();
        assert_eq!(claimed2.id, e2.id);

        // Third claim returns None.
        let third = s.claim_next(r.id, 42, now()).unwrap();
        assert!(third.is_none());
    }

    #[test]
    fn delete_queued_only_works_on_queued() {
        let s = SqliteStore::open_in_memory().unwrap();
        let r = sample_repo();
        s.insert_repo(&r).unwrap();
        let mut e = sample_entry(r.id);
        s.enqueue(&e).unwrap();
        assert!(s.delete_queued(e.id).unwrap());
        assert!(s.get_entry(e.id).unwrap().is_none());

        // Re-insert as Rebasing; delete_queued must refuse.
        e = sample_entry(r.id);
        let mut e_running = e.clone();
        e_running.status = QueueStatus::Rebasing;
        s.enqueue(&e_running).unwrap();
        assert!(!s.delete_queued(e_running.id).unwrap());
        assert!(s.get_entry(e_running.id).unwrap().is_some());
    }

    #[test]
    fn release_returns_in_flight_to_queued() {
        let s = SqliteStore::open_in_memory().unwrap();
        let r = sample_repo();
        s.insert_repo(&r).unwrap();
        let e = sample_entry(r.id);
        s.enqueue(&e).unwrap();
        let claimed = s.claim_next(r.id, 99, now()).unwrap().unwrap();
        assert_eq!(claimed.status, QueueStatus::Rebasing);

        s.release(claimed.id).unwrap();
        let back = s.get_entry(claimed.id).unwrap().unwrap();
        assert_eq!(back.status, QueueStatus::Queued);
        assert_eq!(back.claimed_by_pid, None);
    }

    #[test]
    fn sweep_dead_pid_claims_returns_only_dead_holders_to_queued() {
        let s = SqliteStore::open_in_memory().unwrap();
        let r = sample_repo();
        s.insert_repo(&r).unwrap();
        let e1 = sample_entry(r.id);
        let mut e2 = sample_entry(r.id);
        e2.enqueued_at = e1.enqueued_at + time::Duration::seconds(1);
        s.enqueue(&e1).unwrap();
        s.enqueue(&e2).unwrap();

        // Claim both under different PIDs.
        let c1 = s.claim_next(r.id, 100, now()).unwrap().unwrap();
        let c2 = s.claim_next(r.id, 200, now()).unwrap().unwrap();
        assert_eq!(c1.status, QueueStatus::Rebasing);
        assert_eq!(c2.status, QueueStatus::Rebasing);

        // Pretend PID 100 is dead, PID 200 still alive.
        let n = s.sweep_dead_pid_claims(&[200]).unwrap();
        assert_eq!(n, 1);

        let after1 = s.get_entry(c1.id).unwrap().unwrap();
        let after2 = s.get_entry(c2.id).unwrap().unwrap();
        assert_eq!(after1.status, QueueStatus::Queued);
        assert_eq!(after2.status, QueueStatus::Rebasing);
    }

    #[test]
    fn list_entries_with_status_filter() {
        let s = SqliteStore::open_in_memory().unwrap();
        let r = sample_repo();
        s.insert_repo(&r).unwrap();
        let mut e1 = sample_entry(r.id);
        let mut e2 = sample_entry(r.id);
        e2.enqueued_at = e1.enqueued_at + time::Duration::seconds(1);
        e1.status = QueueStatus::Merged;
        e2.status = QueueStatus::Queued;
        s.enqueue(&e1).unwrap();
        s.enqueue(&e2).unwrap();

        let queued = s
            .list_entries(EntryFilter::all().with_status(vec![QueueStatus::Queued]))
            .unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].id, e2.id);
    }

    #[test]
    fn conflict_session_round_trip() {
        let s = SqliteStore::open_in_memory().unwrap();
        let r = sample_repo();
        s.insert_repo(&r).unwrap();
        let e = sample_entry(r.id);
        s.enqueue(&e).unwrap();

        let cs = ConflictSession {
            id: ConflictSessionId::new(),
            queue_entry_id: e.id,
            agent_backend: AgentBackend::Opencode,
            tmux_session: "mergesmith-test".into(),
            tmux_window: "test-window".into(),
            started_at: now(),
            ended_at: None,
            outcome: None,
        };
        s.open_conflict_session(&cs).unwrap();
        let back = s.get_conflict_session(cs.id).unwrap().unwrap();
        assert_eq!(back.tmux_session, "mergesmith-test");
        assert!(back.outcome.is_none());

        s.close_conflict_session(cs.id, ConflictOutcome::Resolved, now())
            .unwrap();
        let after = s.get_conflict_session(cs.id).unwrap().unwrap();
        assert_eq!(after.outcome, Some(ConflictOutcome::Resolved));
        assert!(after.ended_at.is_some());
    }
}
