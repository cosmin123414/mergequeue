//! Schema migrations: numbered `.sql` files applied in order.

use rusqlite::Connection;
use time::OffsetDateTime;

use crate::error::{Error, Result};

/// The schema version this binary supports.
pub const SUPPORTED_VERSION: i64 = 2;

const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("migrations/0001_init.sql")),
    (2, include_str!("migrations/0002_queue_entry_details.sql")),
];

pub fn run(conn: &mut Connection) -> Result<()> {
    ensure_versions_table(conn)?;

    let current = current_version(conn);
    if current > SUPPORTED_VERSION {
        return Err(Error::SchemaTooNew {
            found: current,
            supported: SUPPORTED_VERSION,
        });
    }

    for (version, sql) in MIGRATIONS {
        if *version <= current {
            continue;
        }
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.execute(
            "INSERT INTO schema_version (version, applied_at) VALUES (?1, ?2)",
            (version, OffsetDateTime::now_utc().unix_timestamp()),
        )?;
        tx.commit()?;
    }
    Ok(())
}

fn ensure_versions_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version    INTEGER PRIMARY KEY,
            applied_at INTEGER NOT NULL
         );",
    )?;
    Ok(())
}

fn current_version(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
        [],
        |r| r.get::<_, i64>(0),
    )
    .unwrap_or(0)
}
